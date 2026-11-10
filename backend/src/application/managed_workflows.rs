//! 托管研究工作流门面：确认冻结合同，驱动生成/执行/修复，并支持可审计重试与恢复。
//!
//! 工作流与普通脚本运行是不同状态机；已确认的输入、引擎快照和结果合同保持不可变。

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crate::adapters::SimEngine;
use crate::agent::knowledge::AgentKnowledge;
use crate::agent::{FailureAnalysis, FailureAnalyzer, ScriptAuthor};
use crate::auth::CurrentUser;
use crate::db::repositories::{
    ManagedWorkflowAggregate, ManagedWorkflowRepo, ManagedWorkflowRepoError,
    ManagedWorkflowStartCommand, NewWorkflowCandidate, RepoError, ScriptRepo,
};
use crate::engine_resources::EngineResourceSnapshot;
use crate::storage::StageLimits;

use super::opaque_token::{generate_token, hash_token, is_generated_token};
use super::struct_overlays::StructOverlayService;
use super::{
    AppEvent, ArtifactMetadata, ArtifactService, EventBus, RunScheduler,
    SessionCancellationRegistry, StudyConfigurationError, StudyConfigurationService,
    WorkflowAttemptCard, WorkflowProgressCard, ms_to_iso, now_ms,
};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmStudyRequest {
    pub expected_revision: i64,
    pub client_operation_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RetryStudyRequest {
    pub client_retry_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConfirmRetryRequest {
    pub confirmed: bool,
}

impl super::PreparedStudyConfirmation {
    pub fn into_start_command(
        self,
        client_operation_id: String,
        correction_budget: i32,
        agent_bundle_version: String,
        agent_skill_refs_json: String,
        created_at_ms: i64,
    ) -> Result<ManagedWorkflowStartCommand, serde_json::Error> {
        let facade_contract_hash = self.locked.facade_contract_hash.clone();
        Ok(ManagedWorkflowStartCommand {
            configuration_id: self.configuration_id,
            session_id: self.session_id,
            configuration_revision: self.expected_revision,
            engine_snapshot_id: self.expected_snapshot_id,
            facade_contract_hash,
            locked_contract_json: serde_json::to_string(&self.locked)?,
            frozen_inputs_json: serde_json::to_string(&self.inputs)?,
            struct_overlays_json: serde_json::to_string(&self.struct_overlays)?,
            result_contract_json: serde_json::to_string(&self.locked.results)?,
            client_operation_id,
            correction_budget,
            agent_bundle_version,
            agent_skill_refs_json,
            created_at_ms,
        })
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryPreviewView {
    pub retry_preview_id: Option<String>,
    pub changes: Value,
    pub expires_at: Option<String>,
    pub workflow: Option<ManagedWorkflowView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAttemptSummary {
    pub id: i32,
    pub attempt_number: i32,
    pub parent_attempt_id: Option<i32>,
    pub phase: String,
    pub status: String,
    pub script_content_hash: Option<String>,
    pub exit_code: Option<i32>,
    pub sanitized_stderr: Option<String>,
    pub diagnostic: Option<Value>,
    pub failure_analysis: Option<FailureAnalysis>,
    pub engine_logs: Vec<WorkflowEngineLog>,
    pub script_changed_from_parent: Option<bool>,
    pub duration_ms: Option<i64>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowEngineLog {
    pub stream: String,
    pub text: String,
    pub total_bytes: u64,
    pub sent_bytes: u64,
    pub truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManagedWorkflowView {
    pub id: i32,
    pub session_id: i32,
    pub configuration_id: Option<i32>,
    pub configuration_revision: Option<i64>,
    pub source_workflow_id: Option<i32>,
    pub status: String,
    pub correction_budget: i32,
    pub corrections_used: i32,
    pub progress_message_id: Option<i32>,
    #[serde(skip)]
    progress_message_revision: i32,
    pub final_script_resource_id: Option<i32>,
    pub run_id: i32,
    pub script_message_id: Option<i32>,
    pub output_dataset_ids: Vec<String>,
    pub error: Option<Value>,
    pub attempts: Vec<WorkflowAttemptSummary>,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
}

#[derive(Debug)]
pub enum WorkflowError {
    NotFound,
    Invalid(&'static str, String),
    WorkflowInProgress(i32),
    StudyConfiguration(StudyConfigurationError),
    Repository(RepoError),
}

impl WorkflowError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound | Self::Repository(RepoError::NotFound) => "not_found",
            Self::Invalid(code, _) => code,
            Self::WorkflowInProgress(_) => "workflow_in_progress",
            Self::StudyConfiguration(error) => error.code(),
            Self::Repository(_) => "workflow_error",
        }
    }

    pub fn workflow_id(&self) -> Option<i32> {
        match self {
            Self::WorkflowInProgress(id) => Some(*id),
            _ => None,
        }
    }
}

impl std::fmt::Display for WorkflowError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "研究工作流不存在"),
            Self::Invalid(_, message) => write!(formatter, "{message}"),
            Self::WorkflowInProgress(_) => write!(formatter, "当前会话已有研究任务正在运行"),
            Self::StudyConfiguration(error) => write!(formatter, "{error}"),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for WorkflowError {}

impl From<StudyConfigurationError> for WorkflowError {
    fn from(value: StudyConfigurationError) -> Self {
        Self::StudyConfiguration(value)
    }
}

#[derive(Clone)]
/// 拥有研究确认、重试、取消和可恢复执行生命周期的应用门面。
pub struct ManagedWorkflowService {
    repo: ManagedWorkflowRepo,
    configurations: StudyConfigurationService,
    events: EventBus,
    artifacts: ArtifactService,
    struct_overlays: StructOverlayService,
    agent_bundle_version: String,
    correction_budget: i32,
    worker: super::workflow_worker::WorkflowWorker,
    auto_start: bool,
    scripts: ScriptRepo,
    snapshot: Arc<EngineResourceSnapshot>,
}

pub(crate) struct ManagedWorkflowDependencies<'a> {
    pub repo: ManagedWorkflowRepo,
    pub configurations: StudyConfigurationService,
    pub events: EventBus,
    pub artifacts: ArtifactService,
    pub struct_overlays: StructOverlayService,
    pub knowledge: &'a AgentKnowledge,
    pub correction_budget: usize,
    pub author: Arc<dyn ScriptAuthor>,
    pub failure_analyzer: Arc<dyn FailureAnalyzer>,
    pub engine: Arc<dyn SimEngine>,
    pub snapshot: Arc<EngineResourceSnapshot>,
    pub data_dir: PathBuf,
    pub scheduler: RunScheduler,
    pub session_cancellations: SessionCancellationRegistry,
    pub engine_timeout: std::time::Duration,
    pub result_max_file_bytes: u64,
    pub auto_start: bool,
    pub scripts: ScriptRepo,
}

impl ManagedWorkflowService {
    pub(crate) fn new(dependencies: ManagedWorkflowDependencies<'_>) -> Self {
        let ManagedWorkflowDependencies {
            repo,
            configurations,
            events,
            artifacts,
            struct_overlays,
            knowledge,
            correction_budget,
            author,
            failure_analyzer,
            engine,
            snapshot,
            data_dir,
            scheduler,
            session_cancellations,
            engine_timeout,
            result_max_file_bytes,
            auto_start,
            scripts,
        } = dependencies;
        let agent_bundle_version = knowledge.manifest.agent_bundle_version.clone();
        let worker = super::workflow_worker::WorkflowWorker::new(
            repo.clone(),
            artifacts.clone(),
            author,
            failure_analyzer,
            engine,
            snapshot.clone(),
            data_dir,
            agent_bundle_version.clone(),
            scheduler,
            session_cancellations,
            engine_timeout,
            result_max_file_bytes,
        );
        Self {
            repo,
            configurations,
            events,
            artifacts,
            struct_overlays,
            agent_bundle_version,
            correction_budget: i32::try_from(correction_budget).unwrap_or(i32::MAX),
            worker,
            auto_start,
            scripts,
            snapshot,
        }
    }

    pub async fn confirm_configuration_and_start(
        &self,
        user: &CurrentUser,
        configuration_id: i32,
        request: ConfirmStudyRequest,
    ) -> Result<ManagedWorkflowView, WorkflowError> {
        let operation = request.client_operation_id.trim();
        if operation.is_empty() || operation.chars().count() > 128 {
            return Err(WorkflowError::Invalid(
                "client_operation_invalid",
                "clientOperationId 必须为 1 到 128 个字符".into(),
            ));
        }
        if let Some(existing) = self
            .repo
            .find_confirmation_replay(
                &user.id,
                configuration_id,
                request.expected_revision,
                operation,
            )
            .await
            .map_err(map_repo_error)?
        {
            return Ok(view(existing));
        }
        let prepared = self
            .configurations
            .prepare_confirmation(user, configuration_id, request.expected_revision)
            .await?;
        let staged = self
            .struct_overlays
            .stage_initial(user, &prepared.struct_overlays)
            .await
            .map_err(|error| WorkflowError::Invalid(error.code(), error.to_string()))?;
        let created_at_ms = now_ms();
        let command = prepared
            .into_start_command(
                operation.to_string(),
                self.correction_budget,
                self.agent_bundle_version.clone(),
                serde_json::to_string(&["study-planning@1.5.0"]).unwrap(),
                created_at_ms,
            )
            .map_err(|error| WorkflowError::Repository(RepoError::Storage(error.to_string())))?;
        let (inputs, drafts, mut staged_artifacts) = staged.into_parts();
        let store = self.artifacts.store().clone();
        let _publication_guard = self.artifacts.exclusive_publication_guard().await;
        let publish_store = store.clone();
        let published_new = Arc::new(Mutex::new(Vec::<String>::new()));
        let published_by_callback = published_new.clone();
        let result = self
            .repo
            .confirm_and_start_with_overlay_artifacts(
                &user.id,
                command,
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
        let (aggregate, should_start) = match result {
            Ok(outcome) => (outcome.aggregate, outcome.created),
            Err(error) => return Err(map_repo_error(error)),
        };
        let view = view(aggregate);
        self.publish(&view);
        if should_start && self.auto_start {
            self.worker.start(user.clone(), view.id, view.session_id);
        }
        Ok(view)
    }

    pub async fn get(
        &self,
        user: &CurrentUser,
        workflow_id: i32,
    ) -> Result<ManagedWorkflowView, WorkflowError> {
        self.repo
            .find(&user.id, workflow_id)
            .await
            .map(view)
            .map_err(map_repo_error)
    }

    pub async fn cancel(
        &self,
        user: &CurrentUser,
        workflow_id: i32,
    ) -> Result<ManagedWorkflowView, WorkflowError> {
        self.worker.cancel(workflow_id);
        let aggregate = self
            .repo
            .cancel(&user.id, workflow_id, now_ms())
            .await
            .map_err(map_repo_error)?;
        let view = view(aggregate);
        self.publish(&view);
        Ok(view)
    }

    pub async fn execute_confirmed(
        &self,
        user: &CurrentUser,
        workflow_id: i32,
    ) -> Result<(), WorkflowError> {
        self.worker
            .run(user, workflow_id)
            .await
            .map_err(|message| WorkflowError::Invalid("workflow_execution_failed", message))
    }

    pub async fn record_candidate_script(
        &self,
        user: &CurrentUser,
        workflow_id: i32,
        phase: &str,
        parent_attempt_id: Option<i32>,
        script: &str,
        agent_model: Option<String>,
    ) -> Result<ManagedWorkflowView, WorkflowError> {
        let staged = self
            .artifacts
            .store()
            .stage(
                std::io::Cursor::new(script.as_bytes()),
                StageLimits {
                    max_bytes: 1024 * 1024,
                },
            )
            .map_err(|error| WorkflowError::Invalid("script_invalid", error.to_string()))?;
        let artifact = self
            .artifacts
            .commit(
                user.storage_scope_id,
                staged,
                ArtifactMetadata {
                    format: "rss".into(),
                    media_type: Some("text/plain".into()),
                },
            )
            .await
            .map_err(|error| WorkflowError::Invalid("script_store_failed", error.to_string()))?;
        let result = self
            .repo
            .record_candidate(
                &user.id,
                workflow_id,
                NewWorkflowCandidate {
                    phase: phase.into(),
                    parent_attempt_id,
                    artifact_id: artifact.id,
                    content_hash: artifact.sha256,
                    agent_bundle_version: self.agent_bundle_version.clone(),
                    agent_skill_refs_json: serde_json::to_string(&["study-planning@1.5.0"])
                        .unwrap(),
                    agent_model,
                    engine_snapshot_id: self.configurations.snapshot_id().to_string(),
                    now_ms: now_ms(),
                },
            )
            .await;
        match result {
            Ok(aggregate) => {
                let view = view(aggregate);
                self.publish(&view);
                Ok(view)
            }
            Err(error) => {
                let _ = self
                    .artifacts
                    .release(user.storage_scope_id, artifact.id)
                    .await;
                Err(map_repo_error(error))
            }
        }
    }

    pub async fn finalize_with_script(
        &self,
        user: &CurrentUser,
        workflow_id: i32,
        attempt_id: i32,
        succeeded: bool,
        error: Option<Value>,
    ) -> Result<ManagedWorkflowView, WorkflowError> {
        let aggregate = self
            .repo
            .finalize_with_script(
                &user.id,
                workflow_id,
                attempt_id,
                succeeded,
                error.map(|value| value.to_string()),
                now_ms(),
            )
            .await
            .map_err(map_repo_error)?;
        let view = view(aggregate);
        self.publish(&view);
        Ok(view)
    }

    fn publish(&self, view: &ManagedWorkflowView) {
        let Some(message_id) = view.progress_message_id else {
            return;
        };
        self.events.publish(
            view.session_id,
            AppEvent::WorkflowProgress {
                id: message_id.to_string(),
                timestamp: view.updated_at.clone(),
                revision: view.progress_message_revision,
                card: progress_card(view),
            },
        );
    }
}

include!("workflow_retry.rs");

fn map_repo_error(error: ManagedWorkflowRepoError) -> WorkflowError {
    match error {
        ManagedWorkflowRepoError::Repository(RepoError::NotFound) => WorkflowError::NotFound,
        ManagedWorkflowRepoError::Repository(error) => WorkflowError::Repository(error),
        ManagedWorkflowRepoError::WorkflowInProgress(id) => WorkflowError::WorkflowInProgress(id),
        ManagedWorkflowRepoError::IdempotencyConflict => WorkflowError::Invalid(
            "idempotency_conflict",
            "clientOperationId 已用于其他确认请求".into(),
        ),
        ManagedWorkflowRepoError::ConfigurationStale => {
            WorkflowError::Invalid("configuration_stale", "研究配置或冻结输入已经变化".into())
        }
        ManagedWorkflowRepoError::TerminalConflict => WorkflowError::Invalid(
            "workflow_terminal_conflict",
            "工作流已经进入其他终态".into(),
        ),
        ManagedWorkflowRepoError::StudyContractChanged => {
            WorkflowError::Invalid("study_contract_changed", "研究门面合同已经变化".into())
        }
        ManagedWorkflowRepoError::RetryInputUnavailable => WorkflowError::Invalid(
            "retry_input_unavailable",
            "重新尝试所需的当前输入不可用".into(),
        ),
        ManagedWorkflowRepoError::InvalidRetryToken => {
            WorkflowError::Invalid("invalid_retry_preview", "重新尝试确认已失效".into())
        }
        ManagedWorkflowRepoError::RetryImpactChanged => WorkflowError::Invalid(
            "retry_impact_changed",
            "重新尝试所使用的输入已经变化".into(),
        ),
    }
}

fn view(aggregate: ManagedWorkflowAggregate) -> ManagedWorkflowView {
    let script_hashes = aggregate
        .attempts
        .iter()
        .map(|attempt| (attempt.id, attempt.script_content_hash.clone()))
        .collect::<std::collections::BTreeMap<_, _>>();
    ManagedWorkflowView {
        id: aggregate.workflow.id,
        session_id: aggregate.workflow.session_id,
        configuration_id: aggregate.workflow.configuration_id,
        configuration_revision: aggregate.workflow.configuration_revision,
        source_workflow_id: aggregate.workflow.source_workflow_id,
        status: aggregate.workflow.status,
        correction_budget: aggregate.workflow.correction_budget,
        corrections_used: aggregate.workflow.corrections_used,
        progress_message_id: aggregate.workflow.progress_message_id,
        progress_message_revision: aggregate.progress_message_revision,
        final_script_resource_id: aggregate.workflow.final_script_resource_id,
        run_id: aggregate.run.id,
        script_message_id: aggregate.run.script_message_id,
        output_dataset_ids: aggregate
            .output_dataset_ids
            .into_iter()
            .map(|id| id.to_string())
            .collect(),
        error: aggregate
            .workflow
            .error_json
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok()),
        attempts: aggregate
            .attempts
            .into_iter()
            .map(|attempt| {
                let diagnostic = attempt
                    .result_diagnostics_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<Value>(value).ok());
                let failure_analysis = diagnostic
                    .as_ref()
                    .and_then(|value| value.get("analysis"))
                    .and_then(|value| serde_json::from_value(value.clone()).ok());
                let engine_logs = ["stdout", "stderr"]
                    .into_iter()
                    .filter_map(|stream| {
                        let value = diagnostic.as_ref()?.get(stream)?;
                        if value.is_null() {
                            return None;
                        }
                        Some(WorkflowEngineLog {
                            stream: stream.into(),
                            text: value.get("text")?.as_str()?.to_string(),
                            total_bytes: value.get("totalBytes")?.as_u64()?,
                            sent_bytes: value.get("sentBytes")?.as_u64()?,
                            truncated: value.get("truncated")?.as_bool()?,
                        })
                    })
                    .collect();
                let script_changed_from_parent = attempt.parent_attempt_id.and_then(|parent_id| {
                    let parent_hash = script_hashes.get(&parent_id)?.as_ref()?;
                    let current_hash = attempt.script_content_hash.as_ref()?;
                    Some(parent_hash != current_hash)
                });
                let duration_ms = attempt
                    .started_at_ms
                    .zip(attempt.finished_at_ms)
                    .map(|(started, finished)| finished.saturating_sub(started));
                WorkflowAttemptSummary {
                    id: attempt.id,
                    attempt_number: attempt.attempt_number,
                    parent_attempt_id: attempt.parent_attempt_id,
                    phase: attempt.phase,
                    status: attempt.status,
                    script_content_hash: attempt.script_content_hash,
                    exit_code: attempt.exit_code,
                    sanitized_stderr: attempt.sanitized_stderr,
                    diagnostic,
                    failure_analysis,
                    engine_logs,
                    script_changed_from_parent,
                    duration_ms,
                    created_at: ms_to_iso(attempt.created_at_ms),
                    started_at: attempt.started_at_ms.map(ms_to_iso),
                    finished_at: attempt.finished_at_ms.map(ms_to_iso),
                }
            })
            .collect(),
        created_at: ms_to_iso(aggregate.workflow.created_at_ms),
        updated_at: ms_to_iso(aggregate.workflow.updated_at_ms),
        finished_at: aggregate.workflow.finished_at_ms.map(ms_to_iso),
    }
}

fn progress_card(view: &ManagedWorkflowView) -> WorkflowProgressCard {
    WorkflowProgressCard {
        workflow_id: view.id.to_string(),
        state: view.status.clone(),
        execution_index: 0,
        execution_max: view.correction_budget.max(0) as u32 + 1,
        repair_index: view.corrections_used.max(0) as u32,
        repair_max: view.correction_budget.max(0) as u32,
        elapsed_ms: 0,
        attempts: view
            .attempts
            .iter()
            .map(|attempt| WorkflowAttemptCard {
                execution_index: attempt.attempt_number.max(0) as u32,
                repair_index: if attempt.phase == "repair" {
                    attempt.attempt_number.saturating_sub(1) as u32
                } else {
                    0
                },
                state: attempt.status.clone(),
                elapsed_ms: 0,
                failure_code: attempt
                    .diagnostic
                    .as_ref()
                    .and_then(|value| value.get("code"))
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
            .collect(),
        script_id: view.final_script_resource_id.map(|id| id.to_string()),
        output_dataset_ids: view.output_dataset_ids.clone(),
        failure_code: view
            .error
            .as_ref()
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::application::{
        CanonicalStructValue, FrozenModelInput, FrozenResultContract, FrozenStudyInput,
        LockedStudyContract, PreparedStructOverlay, PreparedStudyConfirmation,
        WorkflowInputSnapshot,
    };

    #[test]
    fn start_command_preserves_every_locked_confirmation_field() {
        let prepared = PreparedStudyConfirmation {
            configuration_id: 17,
            session_id: 23,
            expected_revision: 5,
            expected_snapshot_id: "snapshot-7".into(),
            struct_overlays: vec![PreparedStructOverlay {
                input_id: "para".into(),
                input_alias: "para".into(),
                source_resource_id: Some("resource-11".into()),
                schema_ref: "frames.para".into(),
                effective_values: json!({"max_iter": 31}),
                user_overrides: json!({"max_iter": 31}),
                selector_overrides: json!({}),
                overrides: json!({"max_iter": 31}),
            }],
            locked: LockedStudyContract {
                configuration_id: 17,
                facade_id: "static_power_flow_newton".into(),
                function: "power_flow".into(),
                facade_contract_hash: "facade-hash".into(),
                engine_snapshot_id: "snapshot-7".into(),
                input_aliases: vec!["bus".into(), "para".into()],
                structs: vec![CanonicalStructValue {
                    input_id: "para".into(),
                    schema_ref: "frames.para".into(),
                    fields: json!({"max_iter": 31}),
                    resource_id: Some("resource-11".into()),
                    resource_content_hash: Some("source-hash".into()),
                    resource_diff: json!({"max_iter": 31}),
                }],
                results: vec![FrozenResultContract {
                    name: "bus_voltage".into(),
                    schema_ref: "results.bus_voltage".into(),
                }],
            },
            inputs: WorkflowInputSnapshot {
                model: Some(FrozenModelInput {
                    model_version_id: 29,
                    content_hash: "model-hash".into(),
                    tables: vec!["bus".into()],
                }),
                resources: vec![FrozenStudyInput {
                    input_id: "para".into(),
                    alias: "para".into(),
                    resource_alias: "para".into(),
                    kind: "struct".into(),
                    resource_id: Some("resource-11".into()),
                    schema_ref: "frames.para".into(),
                    schema_hash: "schema-hash".into(),
                    content_hash: "source-hash".into(),
                }],
            },
        };
        let expected_locked = serde_json::to_value(&prepared.locked).unwrap();
        let expected_inputs = serde_json::to_value(&prepared.inputs).unwrap();
        let expected_overlays = serde_json::to_value(&prepared.struct_overlays).unwrap();

        let command = prepared
            .into_start_command(
                "operation-13".into(),
                3,
                "bundle-4".into(),
                "[\"skill@1\"]".into(),
                101,
            )
            .unwrap();

        assert_eq!(command.configuration_id, 17);
        assert_eq!(command.session_id, 23);
        assert_eq!(command.configuration_revision, 5);
        assert_eq!(command.engine_snapshot_id, "snapshot-7");
        assert_eq!(command.facade_contract_hash, "facade-hash");
        assert_eq!(
            serde_json::from_str::<Value>(&command.locked_contract_json).unwrap(),
            expected_locked
        );
        assert_eq!(
            serde_json::from_str::<Value>(&command.frozen_inputs_json).unwrap(),
            expected_inputs
        );
        assert_eq!(
            serde_json::from_str::<Value>(&command.struct_overlays_json).unwrap(),
            expected_overlays
        );
        assert_eq!(
            serde_json::from_str::<Value>(&command.result_contract_json).unwrap(),
            json!([{"name":"bus_voltage","schemaRef":"results.bus_voltage"}])
        );
        assert_eq!(command.client_operation_id, "operation-13");
        assert_eq!(command.correction_budget, 3);
        assert_eq!(command.agent_bundle_version, "bundle-4");
        assert_eq!(command.agent_skill_refs_json, "[\"skill@1\"]");
        assert_eq!(command.created_at_ms, 101);
    }
}
