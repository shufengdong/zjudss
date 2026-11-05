use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::adapters::{ENGINE_CONTRACT_VERSION, ErrorCategory, RunMode, RunSpec, SimEngine};
use crate::agent::FailureAnalyzer;
use crate::auth::CurrentUser;
use crate::db::models::SimulationRun;
use crate::db::repositories::{
    CreateRun, RepoError, RunDataset, RunDeletionImpact, RunRepo, RunRepoError, ScriptRepo,
};
use crate::engine_resources::EngineResourceSnapshot;

use super::diagnostics::{FailureAnalysisStatus, RunFailureDiagnosis, adapter_engine_logs};
use super::execution_failure::failure_analysis_request;
use super::execution_input_tree::{
    ExecutionInputArtifact, ExecutionInputError, ExecutionInputSpec, build_execution_input_tree,
};
use super::execution_results::inspect_engine_outputs;
use super::run_results::{FrozenResultPolicy, OpenResultPolicy, publish_run_results};
use super::struct_overlays::{StructOverlayError, StructOverlayService};
use super::{
    ArtifactError, ArtifactService, DataResourceError, EventBus, EventWriter, MessageRepo,
    OperationKind, OperationRef, ResultOutputRef, RunScheduler, Stage, StageStatus, ms_to_iso,
    now_ms,
};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[serde(deny_unknown_fields)]
pub struct CreateRunRequest {
    pub client_run_id: String,
}

#[derive(Debug, Clone)]
pub struct RunSubmission {
    pub run: SimulationRun,
    pub script_name: String,
    pub replayed: bool,
}

#[derive(Debug, Clone)]
pub struct RunSummary {
    pub run: SimulationRun,
    pub script_name: String,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDeletionPreview {
    pub impact_token: String,
    #[serde(flatten)]
    pub impact: RunDeletionImpact,
    pub expires_at: String,
}

#[derive(Debug)]
pub enum RunError {
    NotFound,
    ActiveModelChanged,
    ScriptStaticInvalid,
    ScriptIncompatible(Vec<String>),
    ConfigurationStale,
    IdempotencyConflict,
    QueueFull,
    Invalid(String),
    ResultContract(&'static str, String),
    RunBusy,
    ResultBusy,
    InvalidDeletionImpactToken,
    DeletionImpactChanged,
    Artifact(ArtifactError),
    Adapter(crate::adapters::AdapterError),
    Repository(RepoError),
    Io(std::io::Error),
}
impl std::fmt::Display for RunError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "资源不存在"),
            Self::ActiveModelChanged => write!(f, "当前激活模型已经变化"),
            Self::ScriptStaticInvalid => write!(f, "脚本静态校验未通过"),
            Self::ScriptIncompatible(m) => write!(f, "脚本与当前模型不兼容: {}", m.join(",")),
            Self::ConfigurationStale => write!(f, "冻结输入的来源已经变化"),
            Self::IdempotencyConflict => write!(f, "clientRunId 已用于不同运行"),
            Self::QueueFull => write!(f, "运行队列已满"),
            Self::Invalid(v) => write!(f, "{v}"),
            Self::ResultContract(_, v) => write!(f, "{v}"),
            Self::RunBusy => write!(f, "运行尚未结束"),
            Self::ResultBusy => write!(f, "运行结果正在被使用"),
            Self::InvalidDeletionImpactToken => write!(f, "删除确认已失效"),
            Self::DeletionImpactChanged => write!(f, "删除影响已经变化"),
            Self::Artifact(v) => write!(f, "{v}"),
            Self::Adapter(v) => write!(f, "{v}"),
            Self::Repository(v) => write!(f, "{v}"),
            Self::Io(v) => write!(f, "{v}"),
        }
    }
}
impl std::error::Error for RunError {}
impl From<ArtifactError> for RunError {
    fn from(v: ArtifactError) -> Self {
        Self::Artifact(v)
    }
}
impl From<RepoError> for RunError {
    fn from(v: RepoError) -> Self {
        if matches!(v, RepoError::NotFound) {
            Self::NotFound
        } else {
            Self::Repository(v)
        }
    }
}
impl From<RunRepoError> for RunError {
    fn from(v: RunRepoError) -> Self {
        match v {
            RunRepoError::Repository(e) => e.into(),
            RunRepoError::ActiveModelChanged => Self::ActiveModelChanged,
            RunRepoError::ScriptInvalid => Self::ScriptStaticInvalid,
            RunRepoError::ScriptIncompatible(m) => Self::ScriptIncompatible(m),
            RunRepoError::ConfigurationStale => Self::ConfigurationStale,
            RunRepoError::IdempotencyConflict => Self::IdempotencyConflict,
            RunRepoError::RunBusy => Self::RunBusy,
            RunRepoError::ResultBusy => Self::ResultBusy,
            RunRepoError::InvalidDeletionToken => Self::InvalidDeletionImpactToken,
            RunRepoError::DeletionImpactChanged => Self::DeletionImpactChanged,
        }
    }
}

#[derive(Clone)]
pub struct RunService {
    repo: RunRepo,
    scripts: ScriptRepo,
    artifacts: ArtifactService,
    struct_overlays: StructOverlayService,
    engine: Arc<dyn SimEngine>,
    scheduler: RunScheduler,
    messages: MessageRepo,
    events: EventBus,
    session_cancellations: crate::application::SessionCancellationRegistry,
    data_dir: PathBuf,
    snapshot: Arc<EngineResourceSnapshot>,
    engine_timeout: std::time::Duration,
    result_max_file_bytes: u64,
    failure_analyzer: Arc<dyn FailureAnalyzer>,
}

pub(crate) struct RunServiceDependencies {
    pub repo: RunRepo,
    pub scripts: ScriptRepo,
    pub artifacts: ArtifactService,
    pub struct_overlays: StructOverlayService,
    pub engine: Arc<dyn SimEngine>,
    pub scheduler: RunScheduler,
    pub messages: MessageRepo,
    pub events: EventBus,
    pub session_cancellations: crate::application::SessionCancellationRegistry,
    pub data_dir: PathBuf,
    pub snapshot: Arc<EngineResourceSnapshot>,
    pub engine_timeout: std::time::Duration,
    pub result_max_file_bytes: u64,
    pub failure_analyzer: Arc<dyn FailureAnalyzer>,
}

impl RunService {
    pub(crate) fn new(dependencies: RunServiceDependencies) -> Self {
        let RunServiceDependencies {
            repo,
            scripts,
            artifacts,
            struct_overlays,
            engine,
            scheduler,
            messages,
            events,
            session_cancellations,
            data_dir,
            snapshot,
            engine_timeout,
            result_max_file_bytes,
            failure_analyzer,
        } = dependencies;
        Self {
            repo,
            scripts,
            artifacts,
            struct_overlays,
            engine,
            scheduler,
            messages,
            events,
            session_cancellations,
            data_dir,
            snapshot,
            engine_timeout,
            result_max_file_bytes,
            failure_analyzer,
        }
    }

    pub async fn find(&self, user: &CurrentUser, id: i32) -> Result<SimulationRun, RunError> {
        self.repo.find(&user.id, id).await.map_err(Into::into)
    }

    pub async fn find_summary(&self, user: &CurrentUser, id: i32) -> Result<RunSummary, RunError> {
        let run = self.find(user, id).await?;
        let script_name = if let Some(script_id) = run.script_resource_id {
            self.scripts.find(&user.id, script_id).await?.name
        } else {
            "脚本尚未生成".into()
        };
        Ok(RunSummary { run, script_name })
    }

    pub async fn list(
        &self,
        user: &CurrentUser,
        session_id: i32,
        limit: i64,
    ) -> Result<Vec<SimulationRun>, RunError> {
        self.repo
            .list(&user.id, session_id, limit)
            .await
            .map_err(Into::into)
    }

    pub async fn list_summaries(
        &self,
        user: &CurrentUser,
        session_id: i32,
        limit: i64,
    ) -> Result<Vec<RunSummary>, RunError> {
        let runs = self.list(user, session_id, limit).await?;
        let script_names = self
            .scripts
            .list(&user.id, session_id)
            .await?
            .into_iter()
            .map(|script| (script.id, script.name))
            .collect::<BTreeMap<_, _>>();
        runs.into_iter()
            .map(|run| {
                let script_name = match run.script_resource_id {
                    Some(script_id) => script_names
                        .get(&script_id)
                        .cloned()
                        .ok_or(RunError::NotFound)?,
                    None => "脚本尚未生成".into(),
                };
                Ok(RunSummary { run, script_name })
            })
            .collect()
    }

    pub async fn datasets(&self, user: &CurrentUser, id: i32) -> Result<Vec<RunDataset>, RunError> {
        self.repo.datasets(&user.id, id).await.map_err(Into::into)
    }

    pub async fn latest_statuses(
        &self,
        user: &CurrentUser,
    ) -> Result<std::collections::HashMap<i32, String>, RunError> {
        self.repo
            .latest_statuses(&user.id)
            .await
            .map_err(Into::into)
    }
}

include!("run_execution.rs");
include!("run_deletion.rs");

fn public_failure_reason(error: &RunError) -> &'static str {
    match error {
        RunError::Adapter(error) => match error.category {
            ErrorCategory::Model => "model",
            ErrorCategory::Solver => "solver",
            ErrorCategory::Timeout => "timeout",
            ErrorCategory::Io => "io",
            ErrorCategory::Internal => "engine",
        },
        RunError::Invalid(_) | RunError::ResultContract(_, _) => "result",
        RunError::Artifact(_) | RunError::Io(_) => "io",
        _ => "internal",
    }
}

fn map_execution_input_error(error: ExecutionInputError) -> RunError {
    match error {
        ExecutionInputError::Invalid(_, message) => RunError::Invalid(message),
        ExecutionInputError::Artifact(error) => RunError::Artifact(error),
        ExecutionInputError::Io(error) => RunError::Io(error),
    }
}

fn map_rerun_overlay_error(error: StructOverlayError) -> RunError {
    if matches!(
        error.code(),
        "configuration_stale"
            | "not_found"
            | "resource_stale"
            | "structured_schema_mismatch"
            | "structured_values_invalid"
    ) {
        return RunError::ConfigurationStale;
    }
    match error {
        StructOverlayError::Repository(error) => RunError::Repository(error),
        StructOverlayError::Artifact(error) => RunError::Artifact(error),
        StructOverlayError::DataResource(DataResourceError::Repository(error)) => {
            RunError::Repository(error)
        }
        StructOverlayError::DataResource(DataResourceError::Artifact(error)) => {
            RunError::Artifact(error)
        }
        StructOverlayError::DataResource(DataResourceError::Io(error)) => RunError::Io(error),
        error => RunError::Invalid(error.to_string()),
    }
}

fn map_engine_run_error(error: crate::adapters::AdapterError) -> RunError {
    match error.code.as_str() {
        "result_empty" => {
            RunError::ResultContract("no_result_dataset", "脚本没有生成结果数据集".into())
        }
        "result_name_invalid"
        | "result_name_duplicate"
        | "result_format_invalid"
        | "result_compression_invalid" => {
            RunError::ResultContract("result_contract_invalid", error.message)
        }
        _ => RunError::Adapter(error),
    }
}

fn engine_failure_details(error: &RunError) -> (Option<String>, Option<i32>) {
    let RunError::Adapter(error) = error else {
        return (None, None);
    };
    let exit_code = error
        .diagnostics
        .as_deref()
        .and_then(|diagnostics| diagnostics.get("exitCode"))
        .and_then(serde_json::Value::as_i64)
        .and_then(|value| i32::try_from(value).ok());
    let has_process_output = error
        .diagnostics
        .as_deref()
        .is_some_and(|diagnostics| diagnostics.get("exitCode").is_some());
    let output = has_process_output
        .then(|| sanitize_engine_output(&error.message))
        .flatten();
    (output, exit_code)
}

pub(crate) fn sanitize_engine_output(value: &str) -> Option<String> {
    const MAX_ENGINE_OUTPUT_CHARS: usize = 16_384;
    const REDACTED_PATH: &str = "[服务器路径已隐藏]";

    let normalized = value
        .chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\r' | '\t') {
                '�'
            } else {
                character
            }
        })
        .collect::<String>();
    let mut sanitized = String::with_capacity(normalized.len());
    for chunk in normalized.split_inclusive(char::is_whitespace) {
        let body = chunk.trim_end_matches(char::is_whitespace);
        let whitespace = &chunk[body.len()..];
        if contains_absolute_path(body) {
            sanitized.push_str(REDACTED_PATH);
        } else {
            sanitized.push_str(body);
        }
        sanitized.push_str(whitespace);
    }
    let sanitized = sanitized.trim();
    if sanitized.is_empty() {
        return None;
    }
    let mut characters = sanitized.chars();
    let mut bounded = characters
        .by_ref()
        .take(MAX_ENGINE_OUTPUT_CHARS)
        .collect::<String>();
    if characters.next().is_some() {
        bounded.push('…');
    }
    Some(bounded)
}

fn contains_absolute_path(token: &str) -> bool {
    let token = token.trim_start_matches(['"', '\'', '(', '[', '{', '<']);
    if token.starts_with('/') && token.len() > 1 {
        return true;
    }
    let bytes = token.as_bytes();
    bytes.len() > 2
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'/' | b'\\')
}

fn run_stages(active: &str) -> Vec<Stage> {
    [
        ("prepare", "准备输入"),
        ("run", "执行脚本"),
        ("completed", "收集结果"),
    ]
    .into_iter()
    .map(|(key, label)| Stage {
        key: key.into(),
        label: label.into(),
        status: if key == active {
            if active == "completed" {
                StageStatus::Done
            } else {
                StageStatus::Running
            }
        } else if active == "completed" || active == "run" && key == "prepare" {
            StageStatus::Done
        } else {
            StageStatus::Pending
        },
    })
    .collect()
}

fn run_failed_stages(active: &str) -> Vec<Stage> {
    run_stages(active)
        .into_iter()
        .map(|mut stage| {
            if matches!(stage.status, StageStatus::Running) {
                stage.status = StageStatus::Failed;
            }
            stage
        })
        .collect()
}

fn run_output_ref(dataset: &RunDataset) -> ResultOutputRef {
    ResultOutputRef::from_dataset(dataset)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rerun_overlay_errors_report_configuration_stale() {
        for error in [
            DataResourceError::NotFound,
            DataResourceError::Invalid("configuration_stale", "stale".into()),
            DataResourceError::Invalid("structured_schema_mismatch", "schema".into()),
            DataResourceError::Invalid("structured_values_invalid", "values".into()),
        ] {
            assert!(matches!(
                map_rerun_overlay_error(StructOverlayError::DataResource(error)),
                RunError::ConfigurationStale
            ));
        }
    }
}
