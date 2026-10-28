use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::Digest;
use std::path::PathBuf;
use std::sync::Arc;

use crate::adapters::SimEngine;
use crate::agent::ScriptAuthor;
use crate::agent::knowledge::AgentKnowledge;
use crate::auth::CurrentUser;
use crate::db::repositories::{
    ManagedWorkflowAggregate, ManagedWorkflowRepo, ManagedWorkflowRepoError, NewWorkflowCandidate,
    RepoError, ScriptRepo,
};
use crate::engine_resources::EngineResourceSnapshot;
use crate::storage::StageLimits;

use super::{
    AppEvent, ArtifactMetadata, ArtifactService, DataResourceService, EventBus, RunScheduler,
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
    pub created_at: String,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
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
pub struct ManagedWorkflowService {
    repo: ManagedWorkflowRepo,
    configurations: StudyConfigurationService,
    events: EventBus,
    artifacts: ArtifactService,
    data_resources: DataResourceService,
    agent_bundle_version: String,
    correction_budget: i32,
    worker: super::workflow_worker::WorkflowWorker,
    auto_start: bool,
    scripts: ScriptRepo,
    snapshot: Arc<EngineResourceSnapshot>,
}

impl ManagedWorkflowService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: ManagedWorkflowRepo,
        configurations: StudyConfigurationService,
        events: EventBus,
        artifacts: ArtifactService,
        data_resources: DataResourceService,
        knowledge: &AgentKnowledge,
        correction_budget: usize,
        author: Arc<dyn ScriptAuthor>,
        engine: Arc<dyn SimEngine>,
        snapshot: Arc<EngineResourceSnapshot>,
        data_dir: PathBuf,
        scheduler: RunScheduler,
        session_cancellations: SessionCancellationRegistry,
        engine_timeout: std::time::Duration,
        result_max_file_bytes: u64,
        auto_start: bool,
        scripts: ScriptRepo,
    ) -> Self {
        let agent_bundle_version = knowledge.manifest.agent_bundle_version.clone();
        let worker = super::workflow_worker::WorkflowWorker::new(
            repo.clone(),
            artifacts.clone(),
            author,
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
            data_resources,
            agent_bundle_version,
            correction_budget: i32::try_from(correction_budget).unwrap_or(i32::MAX),
            worker,
            auto_start,
            scripts,
            snapshot,
        }
    }

    pub async fn preview_retry(
        &self,
        user: &CurrentUser,
        script_id: i32,
        request: RetryStudyRequest,
    ) -> Result<RetryPreviewView, WorkflowError> {
        let client_retry_id = request.client_retry_id.trim();
        if client_retry_id.is_empty() || client_retry_id.chars().count() > 128 {
            return Err(WorkflowError::Invalid(
                "client_retry_id_invalid",
                "clientRetryId 必须为 1 到 128 个字符".into(),
            ));
        }
        let script = self
            .scripts
            .find(&user.id, script_id)
            .await
            .map_err(WorkflowError::Repository)?;
        let contract: super::LockedStudyContract =
            serde_json::from_str(script.frozen_contract_json.as_deref().ok_or_else(|| {
                WorkflowError::Invalid("retry_not_available", "该脚本不能重新尝试".into())
            })?)
            .map_err(|_| {
                WorkflowError::Invalid("retry_not_available", "脚本研究合同无效".into())
            })?;
        let facade = self
            .snapshot
            .facades
            .get_enabled(&contract.facade_id)
            .ok_or_else(|| {
                WorkflowError::Invalid("study_contract_changed", "研究门面已不可用".into())
            })?;
        if facade.contract_hash != contract.facade_contract_hash {
            return Err(WorkflowError::Invalid(
                "study_contract_changed",
                "研究门面合同已变化".into(),
            ));
        }
        let mut token_bytes = [0_u8; 32];
        getrandom::fill(&mut token_bytes).expect("operating system random source unavailable");
        let token = hex::encode(token_bytes);
        let token_hash = hex::encode(sha2::Sha256::digest(token.as_bytes()));
        let now = now_ms();
        let expires = now + 5 * 60 * 1000;
        let outcome = self
            .repo
            .preview_retry(
                &user.id,
                script_id,
                client_retry_id.into(),
                self.snapshot.id.clone(),
                facade.contract_hash.clone(),
                self.correction_budget,
                self.agent_bundle_version.clone(),
                serde_json::to_string(&["study-planning@1.3.0"]).unwrap(),
                token_hash,
                expires,
                now,
            )
            .await
            .map_err(map_repo_error)?;
        let workflow = outcome.aggregate.map(view);
        if let Some(workflow) = &workflow {
            self.publish(workflow);
            if self.auto_start {
                self.worker
                    .start(user.clone(), workflow.id, workflow.session_id);
            }
        }
        Ok(RetryPreviewView {
            retry_preview_id: workflow.is_none().then_some(token),
            changes: outcome.changes,
            expires_at: workflow.is_none().then(|| ms_to_iso(expires)),
            workflow,
        })
    }

    pub async fn confirm_retry(
        &self,
        user: &CurrentUser,
        retry_preview_id: &str,
        request: ConfirmRetryRequest,
    ) -> Result<ManagedWorkflowView, WorkflowError> {
        if !request.confirmed || retry_preview_id.len() != 64 {
            return Err(WorkflowError::Invalid(
                "invalid_retry_preview",
                "重新尝试确认无效".into(),
            ));
        }
        let token_hash = hex::encode(sha2::Sha256::digest(retry_preview_id.as_bytes()));
        let locked_json = self
            .repo
            .retry_locked_contract(&user.id, &token_hash)
            .await
            .map_err(map_repo_error)?;
        let locked: super::LockedStudyContract =
            serde_json::from_str(&locked_json).map_err(|_| {
                WorkflowError::Invalid("invalid_retry_preview", "重新尝试合同无效".into())
            })?;
        let facade = self
            .snapshot
            .facades
            .get_enabled(&locked.facade_id)
            .ok_or_else(|| {
                WorkflowError::Invalid("study_contract_changed", "研究门面已不可用".into())
            })?;
        if facade.contract_hash != locked.facade_contract_hash {
            return Err(WorkflowError::Invalid(
                "study_contract_changed",
                "研究门面合同已变化".into(),
            ));
        }
        let aggregate = self
            .repo
            .confirm_retry(
                &user.id,
                token_hash,
                self.snapshot.id.clone(),
                facade.contract_hash.clone(),
                self.correction_budget,
                self.agent_bundle_version.clone(),
                serde_json::to_string(&["study-planning@1.3.0"]).unwrap(),
                now_ms(),
            )
            .await
            .map_err(map_repo_error)?;
        let workflow = view(aggregate);
        self.publish(&workflow);
        if self.auto_start {
            self.worker
                .start(user.clone(), workflow.id, workflow.session_id);
        }
        Ok(workflow)
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
        let replacements = self
            .data_resources
            .materialize_study_replacements(user, &prepared)
            .await
            .map_err(|error| WorkflowError::Invalid(error.code(), error.to_string()))?;
        let artifact_ids = replacements
            .iter()
            .map(|replacement| replacement.artifact_id)
            .collect::<Vec<_>>();
        let result = self
            .repo
            .confirm_and_start(
                &user.id,
                prepared,
                replacements,
                operation.to_string(),
                self.correction_budget,
                self.agent_bundle_version.clone(),
                serde_json::to_string(&["study-planning@1.3.0"]).unwrap(),
                now_ms(),
            )
            .await;
        let (aggregate, should_start) = match result {
            Ok(outcome) => {
                if !outcome.replacements_consumed {
                    for artifact_id in artifact_ids {
                        let _ = self
                            .artifacts
                            .release(user.storage_scope_id, artifact_id)
                            .await;
                    }
                }
                (outcome.aggregate, outcome.replacements_consumed)
            }
            Err(error) => {
                for artifact_id in artifact_ids {
                    let _ = self
                        .artifacts
                        .release(user.storage_scope_id, artifact_id)
                        .await;
                }
                return Err(map_repo_error(error));
            }
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
                    agent_skill_refs_json: serde_json::to_string(&["study-planning@1.3.0"])
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
        error: aggregate
            .workflow
            .error_json
            .as_deref()
            .and_then(|value| serde_json::from_str(value).ok()),
        attempts: aggregate
            .attempts
            .into_iter()
            .map(|attempt| WorkflowAttemptSummary {
                id: attempt.id,
                attempt_number: attempt.attempt_number,
                parent_attempt_id: attempt.parent_attempt_id,
                phase: attempt.phase,
                status: attempt.status,
                script_content_hash: attempt.script_content_hash,
                exit_code: attempt.exit_code,
                sanitized_stderr: attempt.sanitized_stderr,
                diagnostic: attempt
                    .result_diagnostics_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str(value).ok()),
                created_at: ms_to_iso(attempt.created_at_ms),
                started_at: attempt.started_at_ms.map(ms_to_iso),
                finished_at: attempt.finished_at_ms.map(ms_to_iso),
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
        output_dataset_ids: Vec::new(),
        failure_code: view
            .error
            .as_ref()
            .and_then(|value| value.get("code"))
            .and_then(Value::as_str)
            .map(str::to_string),
    }
}
