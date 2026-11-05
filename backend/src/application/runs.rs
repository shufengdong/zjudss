use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::adapters::{ENGINE_CONTRACT_VERSION, ErrorCategory, RunMode, RunSpec, SimEngine};
use crate::agent::FailureAnalyzer;
use crate::auth::CurrentUser;
use crate::db::models::SimulationRun;
use crate::db::repositories::{
    ArtifactDraft, CreateRun, NewRunDataset, RepoError, RunArtifactOutputs, RunDataset,
    RunDeletionImpact, RunRepo, RunRepoError, ScriptRepo,
};
use crate::engine_resources::EngineResourceSnapshot;
use crate::storage::{ArtifactStore, StageLimits};

use super::diagnostics::{FailureAnalysisStatus, RunFailureDiagnosis, adapter_engine_logs};
use super::execution_failure::failure_analysis_request;
use super::execution_input_tree::{
    ExecutionInputArtifact, ExecutionInputError, ExecutionInputSpec, build_execution_input_tree,
};
use super::execution_results::inspect_engine_outputs;
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
    pub async fn create(
        &self,
        user: &CurrentUser,
        script_id: i32,
        request: CreateRunRequest,
    ) -> Result<RunSubmission, RunError> {
        if request.client_run_id.trim().is_empty() || request.client_run_id.len() > 128 {
            return Err(RunError::Invalid(
                "clientRunId 必须为 1 到 128 个字符".into(),
            ));
        }
        let script = self.repo_for_script(user, script_id).await?;
        if let Some(created) = self
            .repo
            .find_create_replay(
                &user.id,
                script.session_id,
                script_id,
                &request.client_run_id,
            )
            .await?
        {
            return Ok(RunSubmission {
                run: created.run,
                script_name: script.name,
                replayed: true,
            });
        }
        let ticket = self
            .scheduler
            .try_admit(&user.id, script.session_id)
            .map_err(|_| RunError::QueueFull)?;
        let overlay_requests = self
            .repo
            .struct_overlay_requests(&user.id, script_id)
            .await?;
        let staged = self
            .struct_overlays
            .stage_requests(user, &overlay_requests)
            .await
            .map_err(map_rerun_overlay_error)?;
        let (inputs, drafts, mut staged_artifacts) = staged.into_parts();
        let store = self.artifacts.store().clone();
        let _publication_guard = self.artifacts.exclusive_publication_guard().await;
        let publish_store = store.clone();
        let published_new = Arc::new(Mutex::new(Vec::<String>::new()));
        let published_by_callback = published_new.clone();
        let result = self
            .repo
            .create_with_overlay_artifacts(
                &user.id,
                CreateRun {
                    script_id,
                    client_run_id: request.client_run_id,
                    engine_contract_version: ENGINE_CONTRACT_VERSION.into(),
                    created_at_ms: now_ms(),
                },
                inputs,
                drafts,
                move |index, path| {
                    let existed = publish_store
                        .resolve(path)
                        .map_err(|error| RepoError::Storage(error.to_string()))?
                        .is_file();
                    publish_store
                        .publish(&mut staged_artifacts[index], path)
                        .map_err(|error| RepoError::Storage(error.to_string()))?;
                    if !existed {
                        published_by_callback
                            .lock()
                            .expect("published overlay paths")
                            .push(path.to_string());
                    }
                    Ok(())
                },
            )
            .await;
        if result.is_err() {
            for path in published_new
                .lock()
                .expect("published overlay paths")
                .iter()
            {
                let _ = store.remove(path);
            }
        }
        let created = result?;
        if !created.replayed {
            let service = self.clone();
            let principal = user.clone();
            let run_id = created.run.id;
            let session_id = script.session_id;
            tokio::spawn(async move {
                let mut cancelled = service
                    .session_cancellations
                    .subscribe(&principal.id, session_id);
                if *cancelled.borrow() {
                    return;
                }
                let _ = ticket
                    .run_cancellable(service.execute(principal, run_id), &mut cancelled)
                    .await;
            });
        }
        Ok(RunSubmission {
            run: created.run,
            script_name: script.name,
            replayed: created.replayed,
        })
    }
    async fn repo_for_script(
        &self,
        user: &CurrentUser,
        script_id: i32,
    ) -> Result<crate::db::models::ScriptResource, RunError> {
        self.scripts
            .find(&user.id, script_id)
            .await
            .map_err(Into::into)
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
    pub async fn preview_deletion(
        &self,
        user: &CurrentUser,
        id: i32,
    ) -> Result<RunDeletionPreview, RunError> {
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).expect("operating system random source unavailable");
        let token = hex::encode(bytes);
        let token_hash = hex::encode(Sha256::digest(token.as_bytes()));
        let now = now_ms();
        let expires = now + 5 * 60 * 1000;
        let impact = self
            .repo
            .preview_deletion(&user.id, id, token_hash, expires, now)
            .await?;
        Ok(RunDeletionPreview {
            impact_token: token,
            impact,
            expires_at: ms_to_iso(expires),
        })
    }
    pub async fn delete(
        &self,
        user: &CurrentUser,
        id: i32,
        impact_token: &str,
    ) -> Result<(), RunError> {
        if impact_token.len() != 64 {
            return Err(RunError::InvalidDeletionImpactToken);
        }
        let token_hash = hex::encode(Sha256::digest(impact_token.as_bytes()));
        self.repo
            .delete_run(&user.id, id, token_hash, now_ms())
            .await?;
        Ok(())
    }
    async fn execute(&self, user: CurrentUser, id: i32) {
        if let Err(error) = self.execute_inner(&user, id).await {
            let run = self.repo.find(&user.id, id).await.ok();
            let failed_stage = run
                .as_ref()
                .map(|run| run.stage.clone())
                .unwrap_or_else(|| "run".to_string());
            eprintln!("[run] failed id={id}, stage={failed_stage}: {error}");
            let reason = public_failure_reason(&error);
            let (_, exit_code) = engine_failure_details(&error);
            let code = match &error {
                RunError::ResultContract(code, _) => *code,
                _ => "run_failed",
            };
            let finished_at = now_ms();
            let engine_logs = match &error {
                RunError::Adapter(error) => adapter_engine_logs(error),
                _ => Vec::new(),
            };
            let failure = RunFailureDiagnosis {
                code: code.into(),
                reason: reason.into(),
                stage: failed_stage.clone(),
                duration_ms: run
                    .as_ref()
                    .and_then(|run| run.started_at_ms)
                    .map(|started| finished_at.saturating_sub(started)),
                engine_logs,
                analysis_status: if matches!(error, RunError::Adapter(_)) {
                    FailureAnalysisStatus::Pending
                } else {
                    FailureAnalysisStatus::Unavailable
                },
                analysis: None,
            };
            let failed = self
                .repo
                .fail(
                    &user.id,
                    id,
                    &failed_stage,
                    serde_json::to_string(&failure)
                        .expect("bounded run failure diagnosis serializes"),
                    exit_code,
                    finished_at,
                )
                .await;
            if failed.is_ok()
                && let Some(run) = run
            {
                let writer = EventWriter::new(
                    user.id.clone(),
                    run.session_id,
                    self.messages.clone(),
                    self.events.clone(),
                );
                let _ = writer
                    .progress(
                        OperationRef {
                            kind: OperationKind::Run,
                            id: id.to_string(),
                        },
                        run_failed_stages(&failed_stage),
                    )
                    .await;
                if let RunError::Adapter(adapter_error) = error {
                    let service = self.clone();
                    let principal = user.clone();
                    tokio::spawn(async move {
                        service
                            .enrich_failure_analysis(principal, run, adapter_error)
                            .await;
                    });
                }
            }
        }
    }

    async fn enrich_failure_analysis(
        &self,
        user: CurrentUser,
        run: SimulationRun,
        error: crate::adapters::AdapterError,
    ) {
        let analysis = async {
            let artifact_id = run.script_artifact_id?;
            let artifact = self
                .artifacts
                .find_owned(user.storage_scope_id, artifact_id)
                .await
                .ok()?;
            let path = self
                .artifacts
                .store()
                .resolve(&artifact.storage_path)
                .ok()?;
            let current_script = std::fs::read_to_string(path).ok()?;
            let request = failure_analysis_request(current_script, &error, None, None);
            self.failure_analyzer.analyze_failure(request).await.ok()
        };
        let analysis = tokio::time::timeout(std::time::Duration::from_secs(30), analysis)
            .await
            .ok()
            .flatten();
        let (status, analysis) = match analysis.as_ref() {
            Some(analysis) => (FailureAnalysisStatus::Complete, Some(analysis)),
            None => (FailureAnalysisStatus::Unavailable, None),
        };
        let _ = self
            .repo
            .complete_failure_analysis(&user.id, run.id, status, analysis)
            .await;
    }
    async fn execute_inner(&self, user: &CurrentUser, id: i32) -> Result<(), RunError> {
        self.repo
            .set_running(&user.id, id, "prepare", now_ms())
            .await?;
        let writer = EventWriter::new(
            user.id.clone(),
            self.repo.find(&user.id, id).await?.session_id,
            self.messages.clone(),
            self.events.clone(),
        );
        let _ = writer
            .progress(
                OperationRef {
                    kind: OperationKind::Run,
                    id: id.to_string(),
                },
                run_stages("prepare"),
            )
            .await;
        let context = self.repo.context(&user.id, id).await?;
        let script_artifact_id = context
            .run
            .script_artifact_id
            .ok_or_else(|| RunError::Invalid("受管脚本尚未生成".into()))?;
        let tree = build_execution_input_tree(
            user,
            &self.artifacts,
            &self.data_dir,
            "runs",
            id,
            ExecutionInputSpec {
                script_artifact_id,
                model_frames: context
                    .frames
                    .into_iter()
                    .map(|frame| ExecutionInputArtifact {
                        alias: frame.table_name,
                        artifact_id: frame.artifact_id,
                    })
                    .collect(),
                resources: Vec::new(),
            },
        )
        .await
        .map_err(map_execution_input_error)?;
        self.repo.set_running(&user.id, id, "run", now_ms()).await?;
        let _ = writer
            .progress(
                OperationRef {
                    kind: OperationKind::Run,
                    id: id.to_string(),
                },
                run_stages("run"),
            )
            .await;
        let output = self
            .engine
            .run_dataframe(RunSpec {
                mode: RunMode::Dataframe,
                input_tree: tree.input_dir.clone(),
                script_path: tree.script_path.clone(),
                named_inputs: tree.named_inputs.clone(),
                result_dir: tree.result_dir.clone(),
                timeout: self.engine_timeout,
            })
            .await
            .map_err(map_engine_run_error)?;
        if output.datasets.is_empty() {
            return Err(RunError::ResultContract(
                "no_result_dataset",
                "脚本没有生成结果数据集".into(),
            ));
        }
        let inspected = inspect_engine_outputs(&output.datasets, self.result_max_file_bytes)
            .map_err(|error| RunError::Invalid(error.to_string()))?;
        let actual_names = inspected
            .iter()
            .map(|dataset| dataset.name.as_str())
            .collect::<std::collections::BTreeSet<_>>();
        if actual_names.len() != inspected.len()
            || actual_names.iter().any(|name| !valid_result_name(name))
        {
            return Err(RunError::ResultContract(
                "result_contract_invalid",
                "结果名称不合法或重复".into(),
            ));
        }
        if let Some(contract_json) = context.run.result_contract_json.as_deref() {
            let contract: Vec<super::FrozenResultContract> = serde_json::from_str(contract_json)
                .map_err(|_| {
                    RunError::ResultContract("result_contract_invalid", "冻结结果合同无效".into())
                })?;
            let expected = contract
                .iter()
                .map(|result| result.name.as_str())
                .collect::<std::collections::BTreeSet<_>>();
            if expected != actual_names || contract.len() != inspected.len() {
                return Err(RunError::ResultContract(
                    "result_contract_invalid",
                    "结果名称集合与冻结合同不一致".into(),
                ));
            }
            for expected in contract {
                let dataset = inspected
                    .iter()
                    .find(|dataset| dataset.name == expected.name)
                    .expect("name set checked");
                let schema = self
                    .snapshot
                    .schemas
                    .resolve(&expected.schema_ref)
                    .map_err(|_| {
                        RunError::ResultContract(
                            "result_contract_invalid",
                            "冻结结果 schema 不可用".into(),
                        )
                    })?;
                super::workflow_worker::validate_schema(&dataset.metadata, &schema.fields)
                    .map_err(|message| {
                        RunError::ResultContract("result_contract_invalid", message)
                    })?;
                super::workflow_worker::validate_non_null_values(&dataset.path, &schema.fields)
                    .map_err(|message| {
                        RunError::ResultContract("result_contract_invalid", message)
                    })?;
            }
        }
        let mut staged_artifacts = Vec::new();
        let mut drafts = Vec::new();
        let mut datasets = Vec::new();
        for output in inspected {
            let metadata = output.metadata;
            let staged = self
                .artifacts
                .store()
                .stage(
                    std::fs::File::open(&output.path).map_err(RunError::Io)?,
                    StageLimits {
                        max_bytes: self.result_max_file_bytes,
                    },
                )
                .map_err(ArtifactError::from)?;
            let size_bytes = i64::try_from(staged.size_bytes)
                .map_err(|_| RunError::Invalid("结果过大".into()))?;
            drafts.push(ArtifactDraft {
                sha256: staged.sha256.clone(),
                size_bytes,
                storage_path: ArtifactStore::relative_path(&staged.sha256),
                format: "parquet".into(),
                media_type: Some("application/vnd.apache.parquet".into()),
                created_at_ms: now_ms(),
            });
            staged_artifacts.push(staged);
            let (schema_json, schema_hash) =
                crate::application::datasets::schema_json_and_hash(&metadata);
            datasets.push(NewRunDataset {
                artifact_id: 0,
                result_name: output.name,
                schema_json,
                schema_hash,
                row_count: metadata.row_count as i64,
                size_bytes: metadata.size_bytes as i64,
                created_at_ms: now_ms(),
            });
        }
        let store = self.artifacts.store().clone();
        let _publication_guard = self.artifacts.exclusive_publication_guard().await;
        let publish_store = store.clone();
        let published_new = Arc::new(Mutex::new(Vec::<String>::new()));
        let published_by_callback = published_new.clone();
        let publication = self
            .repo
            .complete_with_artifact_outputs(
                &user.id,
                RunArtifactOutputs {
                    scope_id: user.storage_scope_id,
                    run_id: id,
                    drafts,
                    datasets,
                    finished_at_ms: now_ms(),
                },
                move |index, path| {
                    let existed = publish_store
                        .resolve(path)
                        .map_err(|error| RepoError::Storage(error.to_string()))?
                        .is_file();
                    publish_store
                        .publish(&mut staged_artifacts[index], path)
                        .map_err(|error| RepoError::Storage(error.to_string()))?;
                    if !existed {
                        published_by_callback
                            .lock()
                            .expect("published run paths")
                            .push(path.to_string());
                    }
                    Ok(())
                },
            )
            .await;
        if publication.is_err() {
            for path in published_new.lock().expect("published run paths").iter() {
                let _ = store.remove(path);
            }
        }
        match publication {
            Ok(results) => {
                let _ = writer
                    .progress(
                        OperationRef {
                            kind: OperationKind::Run,
                            id: id.to_string(),
                        },
                        run_stages("completed"),
                    )
                    .await;
                let _ = writer
                    .result(
                        Some(id),
                        None,
                        "运行结果",
                        results.iter().map(run_output_ref).collect(),
                    )
                    .await;
                Ok(())
            }
            Err(error) => Err(error.into()),
        }
    }
}

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

fn valid_result_name(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('a'..='z' | 'A'..='Z' | '_'))
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
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
