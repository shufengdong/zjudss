use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::watch;

use crate::adapters::{AdapterError, NamedInput, RunMode, RunSpec, SimEngine};
use crate::agent::{EngineDocRef, ScriptAuthor, ScriptGenerationRequest, ScriptRepairRequest};
use crate::auth::CurrentUser;
use crate::catalog::{DataTypeContract, SchemaFieldContract};
use crate::db::repositories::{
    ArtifactDraft, ManagedWorkflowAggregate, ManagedWorkflowRepo, NewWorkflowCandidate,
    PendingRunDataset, WorkflowAttemptUpdate,
};
use crate::engine_resources::EngineResourceSnapshot;
use crate::storage::{ArtifactStore, ParquetMetadata, StageLimits};

use super::diagnostics::{SanitizedDiagnostic, raw_adapter_stderr, sanitize_adapter_stderr};
use super::script_contract::ScriptContractInspector;
use super::{
    ArtifactError, ArtifactMetadata, ArtifactService, LockedStudyContract, RunScheduler,
    SessionCancellationRegistry, WorkflowInputSnapshot, now_ms,
};

#[derive(Clone)]
pub struct WorkflowWorker {
    repo: ManagedWorkflowRepo,
    artifacts: ArtifactService,
    author: Arc<dyn ScriptAuthor>,
    engine: Arc<dyn SimEngine>,
    snapshot: Arc<EngineResourceSnapshot>,
    data_dir: PathBuf,
    agent_bundle_version: String,
    scheduler: RunScheduler,
    session_cancellations: SessionCancellationRegistry,
    engine_timeout: std::time::Duration,
    result_max_file_bytes: u64,
    cancellations: Arc<Mutex<BTreeMap<i32, watch::Sender<bool>>>>,
}

impl WorkflowWorker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: ManagedWorkflowRepo,
        artifacts: ArtifactService,
        author: Arc<dyn ScriptAuthor>,
        engine: Arc<dyn SimEngine>,
        snapshot: Arc<EngineResourceSnapshot>,
        data_dir: PathBuf,
        agent_bundle_version: String,
        scheduler: RunScheduler,
        session_cancellations: SessionCancellationRegistry,
        engine_timeout: std::time::Duration,
        result_max_file_bytes: u64,
    ) -> Self {
        let _ = std::fs::remove_dir_all(data_dir.join("managed-workflows"));
        Self {
            repo,
            artifacts,
            author,
            engine,
            snapshot,
            data_dir,
            agent_bundle_version,
            scheduler,
            session_cancellations,
            engine_timeout,
            result_max_file_bytes,
            cancellations: Arc::new(Mutex::new(BTreeMap::new())),
        }
    }

    pub fn start(&self, user: CurrentUser, workflow_id: i32, session_id: i32) {
        let worker = self.clone();
        let ticket = worker.scheduler.try_admit(&user.id, session_id);
        let (cancel_tx, mut workflow_cancel) = watch::channel(false);
        let session_cancel_tx = cancel_tx.clone();
        worker
            .cancellations
            .lock()
            .expect("workflow cancellation registry")
            .insert(workflow_id, cancel_tx);
        tokio::spawn(async move {
            let Ok(ticket) = ticket else {
                let _ = worker
                    .finish_failure(
                        &user,
                        workflow_id,
                        None,
                        "agent_queue_full",
                        "Agent 请求队列已满，请稍后重试",
                    )
                    .await;
                worker.remove_cancellation(workflow_id);
                return;
            };
            let mut session_cancelled =
                worker.session_cancellations.subscribe(&user.id, session_id);
            if *session_cancelled.borrow() {
                session_cancel_tx.send_replace(true);
                worker.remove_cancellation(workflow_id);
                return;
            }
            let session_forward = tokio::spawn(async move {
                let _ = session_cancelled.changed().await;
                session_cancel_tx.send_replace(true);
            });
            let mut execution_cancel = workflow_cancel.clone();
            let _ = ticket
                .run_cancellable(
                    worker.run_loop(&user, workflow_id, &mut execution_cancel),
                    &mut workflow_cancel,
                )
                .await;
            session_forward.abort();
            worker.remove_cancellation(workflow_id);
        });
    }

    pub fn cancel(&self, workflow_id: i32) {
        if let Some(sender) = self
            .cancellations
            .lock()
            .expect("workflow cancellation registry")
            .get(&workflow_id)
        {
            let _ = sender.send(true);
        }
    }

    pub async fn run(&self, user: &CurrentUser, workflow_id: i32) -> Result<(), String> {
        let (cancel_tx, mut cancel_rx) = watch::channel(false);
        self.cancellations
            .lock()
            .expect("workflow cancellation registry")
            .insert(workflow_id, cancel_tx);
        let result = self.run_loop(user, workflow_id, &mut cancel_rx).await;
        self.remove_cancellation(workflow_id);
        result
    }

    fn remove_cancellation(&self, workflow_id: i32) {
        self.cancellations
            .lock()
            .expect("workflow cancellation registry")
            .remove(&workflow_id);
    }

    async fn run_loop(
        &self,
        user: &CurrentUser,
        workflow_id: i32,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<(), String> {
        let mut aggregate = self
            .repo
            .find(&user.id, workflow_id)
            .await
            .map_err(|error| error.to_string())?;
        let locked: LockedStudyContract = serde_json::from_str(
            aggregate
                .workflow
                .locked_contract_json
                .as_deref()
                .ok_or("workflow locked contract missing")?,
        )
        .map_err(|error| error.to_string())?;
        let inputs: WorkflowInputSnapshot =
            serde_json::from_str(&aggregate.run.input_snapshot_json)
                .map_err(|error| error.to_string())?;
        let mut parent_attempt_id = aggregate.attempts.last().map(|attempt| attempt.id);
        let mut current_script = String::new();
        let mut repair_diagnostic = None;
        let mut repair_stderr = String::new();
        let mut phase = "generation";

        loop {
            if cancelled(cancel, &aggregate) {
                return Ok(());
            }
            if phase == "repair"
                && aggregate.workflow.corrections_used >= aggregate.workflow.correction_budget
            {
                self.finish_failure(
                    user,
                    workflow_id,
                    parent_attempt_id,
                    "repair_exhausted",
                    "脚本自动修正次数已用尽",
                )
                .await?;
                return Ok(());
            }
            let authored = if phase == "generation" {
                let request = ScriptGenerationRequest {
                    locked: locked.clone(),
                    inputs: inputs.clone(),
                    engine_docs: self.engine_docs(&locked.function),
                    instruction: "所有冻结 alias 已由运行环境注入。只写一条直接结果绑定，严格按冻结结果名、函数名和输入顺序调用；不要读取或重新赋值任何输入。只输出完整 RSS 脚本，不要解释。".into(),
                };
                tokio::select! {
                    result = self.author.generate(request) => result,
                    changed = cancel.changed() => {
                        let _ = changed;
                        return Ok(());
                    }
                }
            } else {
                let request = ScriptRepairRequest {
                    current_script: current_script.clone(),
                    sanitized_stderr: repair_stderr.clone(),
                    diagnostic: repair_diagnostic.clone(),
                    locked_contract: locked.clone(),
                    inputs: inputs.clone(),
                    instruction: "必须依据 diagnostic 和 sanitizedStderr 修改 currentScript；冻结 alias 已由运行环境注入，不得读取、构造或重新赋值。只输出修改后的完整脚本，不要解释".into(),
                };
                tokio::select! {
                    result = self.author.repair(request) => result,
                    changed = cancel.changed() => {
                        let _ = changed;
                        return Ok(());
                    }
                }
            };
            let script = match authored {
                Ok(script) => script,
                Err(error) => {
                    let code = if phase == "generation" {
                        "script_generation_model_unavailable"
                    } else {
                        "repair_model_unavailable"
                    };
                    self.finish_failure(
                        user,
                        workflow_id,
                        parent_attempt_id,
                        code,
                        &error.to_string(),
                    )
                    .await?;
                    return Ok(());
                }
            };
            if *cancel.borrow() {
                return Ok(());
            }
            aggregate = match self
                .store_candidate(user, workflow_id, phase, parent_attempt_id, &script)
                .await
            {
                Ok(aggregate) => aggregate,
                Err(error) => {
                    self.finish_failure(
                        user,
                        workflow_id,
                        parent_attempt_id,
                        error.code,
                        &error.message,
                    )
                    .await?;
                    return Ok(());
                }
            };
            let attempt = aggregate
                .attempts
                .last()
                .cloned()
                .ok_or("candidate attempt missing")?;
            parent_attempt_id = Some(attempt.id);
            current_script = script;

            if let Err(violation) =
                ScriptContractInspector::inspect_candidate(&current_script, &locked)
            {
                let diagnostic = violation.diagnostic;
                aggregate = self
                    .update_attempt(
                        user,
                        workflow_id,
                        attempt.id,
                        "contract_rejected",
                        "repairing",
                        None,
                        None,
                        None,
                        Some(diagnostic.clone()),
                    )
                    .await?;
                repair_stderr.clear();
                repair_diagnostic = Some(diagnostic);
                phase = "repair";
                continue;
            }

            aggregate = self
                .update_attempt(
                    user,
                    workflow_id,
                    attempt.id,
                    "executing",
                    "executing",
                    None,
                    None,
                    None,
                    None,
                )
                .await?;
            let execution = self
                .execute_candidate(user, &aggregate, &current_script, cancel)
                .await;
            match execution {
                Ok(outputs) => match self.validate_results(&locked, &outputs) {
                    Ok(()) => {
                        let published = self
                            .publish_results(user, workflow_id, attempt.id, outputs)
                            .await;
                        match published {
                            Ok(()) => return Ok(()),
                            Err(error) => {
                                self.finish_failure(
                                    user,
                                    workflow_id,
                                    Some(attempt.id),
                                    error.code,
                                    &error.message,
                                )
                                .await?;
                                return Ok(());
                            }
                        }
                    }
                    Err(diagnostic) => {
                        cleanup_outputs(&outputs);
                        aggregate = self
                            .update_attempt(
                                user,
                                workflow_id,
                                attempt.id,
                                "result_rejected",
                                "repairing",
                                Some(0),
                                None,
                                None,
                                Some(diagnostic.clone()),
                            )
                            .await?;
                        repair_stderr.clear();
                        repair_diagnostic = Some(diagnostic);
                        phase = "repair";
                    }
                },
                Err(ExecutionFailure::Cancelled) => return Ok(()),
                Err(ExecutionFailure::Engine(error)) => {
                    let sanitized = sanitize_adapter_stderr(&error);
                    let audit_artifact_id = match self.store_stderr_audit(user, &error).await {
                        Ok(artifact_id) => artifact_id,
                        Err(message) => {
                            self.finish_failure(
                                user,
                                workflow_id,
                                Some(attempt.id),
                                "stderr_audit_failed",
                                &message,
                            )
                            .await?;
                            return Ok(());
                        }
                    };
                    let exit_code = exit_code(&error);
                    let diagnostic = public_engine_diagnostic(&error, &sanitized);
                    let repairable = repairable_engine_error(&error.code, exit_code);
                    let updated = self
                        .update_attempt(
                            user,
                            workflow_id,
                            attempt.id,
                            "engine_failed",
                            if repairable { "repairing" } else { "executing" },
                            exit_code,
                            Some(sanitized.clone()),
                            Some(audit_artifact_id),
                            Some(diagnostic.clone()),
                        )
                        .await;
                    aggregate = match updated {
                        Ok(aggregate) => aggregate,
                        Err(message) => {
                            let _ = self
                                .artifacts
                                .release(user.storage_scope_id, audit_artifact_id)
                                .await;
                            return Err(message);
                        }
                    };
                    if !repairable {
                        self.finish_failure(
                            user,
                            workflow_id,
                            Some(attempt.id),
                            &error.code,
                            "仿真引擎执行失败，请查看已脱敏诊断",
                        )
                        .await?;
                        return Ok(());
                    }
                    repair_stderr = sanitized.text;
                    repair_diagnostic = Some(diagnostic);
                    phase = "repair";
                }
                Err(ExecutionFailure::Local(code, message)) => {
                    self.finish_failure(user, workflow_id, Some(attempt.id), code, &message)
                        .await?;
                    return Ok(());
                }
            }
        }
    }

    fn engine_docs(&self, function: &str) -> Vec<EngineDocRef> {
        self.snapshot
            .knowledge
            .search(function, 6_000)
            .into_iter()
            .map(|hit| EngineDocRef {
                id: format!("{}#{}", hit.doc_id, hit.section_id),
                content_hash: hit.hash,
                excerpt: hit.snippet,
            })
            .collect()
    }

    async fn store_candidate(
        &self,
        user: &CurrentUser,
        workflow_id: i32,
        phase: &str,
        parent_attempt_id: Option<i32>,
        script: &str,
    ) -> Result<ManagedWorkflowAggregate, CandidateStoreFailure> {
        let staged = self
            .artifacts
            .store()
            .stage(
                Cursor::new(script.as_bytes()),
                StageLimits {
                    max_bytes: 1024 * 1024,
                },
            )
            .map_err(|_error| CandidateStoreFailure {
                code: "script_candidate_store_failed",
                message: "候选脚本保存失败".into(),
            })?;
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
            .map_err(|error| CandidateStoreFailure {
                code: if matches!(error, ArtifactError::QuotaExceeded) {
                    "storage_quota_exceeded"
                } else {
                    "script_candidate_store_failed"
                },
                message: if matches!(error, ArtifactError::QuotaExceeded) {
                    "存储配额不足".into()
                } else {
                    "候选脚本保存失败".into()
                },
            })?;
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
                        .expect("static skill refs serialize"),
                    agent_model: None,
                    engine_snapshot_id: self.snapshot.id.clone(),
                    now_ms: now_ms(),
                },
            )
            .await;
        match result {
            Ok(aggregate) => Ok(aggregate),
            Err(_error) => {
                let _ = self
                    .artifacts
                    .release(user.storage_scope_id, artifact.id)
                    .await;
                Err(CandidateStoreFailure {
                    code: "script_candidate_store_failed",
                    message: "候选脚本保存失败".into(),
                })
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn update_attempt(
        &self,
        user: &CurrentUser,
        workflow_id: i32,
        attempt_id: i32,
        status: &str,
        workflow_status: &str,
        exit_code: Option<i32>,
        sanitized: Option<SanitizedDiagnostic>,
        stderr_audit_artifact_id: Option<i32>,
        diagnostic: Option<Value>,
    ) -> Result<ManagedWorkflowAggregate, String> {
        let executing = status == "executing";
        let finished = matches!(
            status,
            "contract_rejected" | "engine_failed" | "result_rejected"
        );
        self.repo
            .update_attempt(
                &user.id,
                workflow_id,
                attempt_id,
                WorkflowAttemptUpdate {
                    status: status.into(),
                    workflow_status: workflow_status.into(),
                    exit_code,
                    stderr_audit_artifact_id,
                    sanitized_stderr: sanitized.as_ref().map(|value| value.text.clone()),
                    stderr_full_hash: sanitized.as_ref().map(|value| value.full_hash.clone()),
                    stderr_retained_hash: sanitized
                        .as_ref()
                        .map(|value| value.retained_hash.clone()),
                    stderr_sent_hash: sanitized.as_ref().map(|value| value.sent_hash.clone()),
                    stderr_total_bytes: sanitized
                        .as_ref()
                        .and_then(|value| i64::try_from(value.total_bytes).ok()),
                    stderr_sent_bytes: sanitized
                        .as_ref()
                        .and_then(|value| i64::try_from(value.sent_bytes).ok()),
                    stderr_truncated: sanitized.as_ref().is_some_and(|value| value.truncated),
                    diagnostic_json: diagnostic.map(|value| value.to_string()),
                    started: executing,
                    finished,
                    now_ms: now_ms(),
                },
            )
            .await
            .map_err(|error| error.to_string())
    }

    async fn store_stderr_audit(
        &self,
        user: &CurrentUser,
        error: &AdapterError,
    ) -> Result<i32, String> {
        let raw = raw_adapter_stderr(error);
        let staged = self
            .artifacts
            .store()
            .stage(
                Cursor::new(raw),
                StageLimits {
                    max_bytes: 1024 * 1024,
                },
            )
            .map_err(|error| error.to_string())?;
        self.artifacts
            .commit(
                user.storage_scope_id,
                staged,
                ArtifactMetadata {
                    format: "engine-stderr".into(),
                    media_type: Some("application/octet-stream".into()),
                },
            )
            .await
            .map(|artifact| artifact.id)
            .map_err(|error| error.to_string())
    }

    async fn execute_candidate(
        &self,
        user: &CurrentUser,
        aggregate: &ManagedWorkflowAggregate,
        script: &str,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<Vec<crate::adapters::EngineDatasetOutput>, ExecutionFailure> {
        let attempt = aggregate.attempts.last().ok_or_else(|| {
            ExecutionFailure::Local("attempt_missing", "运行 attempt 不存在".into())
        })?;
        let root = self
            .data_dir
            .join("managed-workflows")
            .join(aggregate.workflow.id.to_string())
            .join(attempt.id.to_string());
        let mut private_tree = PrivateTree::new(root.clone());
        let input = root.join("input");
        let result = root.join("result");
        std::fs::create_dir_all(&input)
            .and_then(|_| std::fs::create_dir_all(&result))
            .map_err(|error| ExecutionFailure::Local("input_stage_failed", error.to_string()))?;
        let script_path = PathBuf::from("script.rss");
        std::fs::write(input.join(&script_path), script)
            .map_err(|error| ExecutionFailure::Local("input_stage_failed", error.to_string()))?;
        let mut named_inputs = Vec::new();
        for frozen in &aggregate.inputs {
            let artifact_id = frozen.artifact_id.ok_or_else(|| {
                ExecutionFailure::Local("input_artifact_missing", frozen.input_name.clone())
            })?;
            let alias = frozen.input_alias.clone().ok_or_else(|| {
                ExecutionFailure::Local("input_alias_missing", frozen.input_name.clone())
            })?;
            let relative = PathBuf::from("inputs").join(format!("{alias}.parquet"));
            std::fs::create_dir_all(input.join("inputs")).map_err(|error| {
                ExecutionFailure::Local("input_stage_failed", error.to_string())
            })?;
            let mut source = self
                .artifacts
                .open_owned(user.storage_scope_id, artifact_id)
                .await
                .map_err(|error| {
                    ExecutionFailure::Local("input_artifact_missing", error.to_string())
                })?;
            let mut target = std::fs::File::create(input.join(&relative)).map_err(|error| {
                ExecutionFailure::Local("input_stage_failed", error.to_string())
            })?;
            std::io::copy(&mut source, &mut target).map_err(|error| {
                ExecutionFailure::Local("input_stage_failed", error.to_string())
            })?;
            named_inputs.push(NamedInput {
                alias,
                relative_path: relative,
            });
        }
        let run = self.engine.run_dataframe(RunSpec {
            mode: RunMode::Dataframe,
            input_tree: input,
            script_path,
            named_inputs,
            result_dir: result,
            timeout: self.engine_timeout,
        });
        tokio::pin!(run);
        let outcome = tokio::select! {
            result = &mut run => result.map(|output| output.datasets).map_err(ExecutionFailure::Engine),
            changed = cancel.changed() => {
                let _ = changed;
                Err(ExecutionFailure::Cancelled)
            }
        };
        if outcome.is_ok() {
            private_tree.keep();
        }
        outcome
    }

    fn validate_results(
        &self,
        locked: &LockedStudyContract,
        outputs: &[crate::adapters::EngineDatasetOutput],
    ) -> Result<(), Value> {
        let expected = locked
            .results
            .iter()
            .map(|result| result.name.as_str())
            .collect::<BTreeSet<_>>();
        let actual = outputs
            .iter()
            .map(|output| output.name.as_str())
            .collect::<BTreeSet<_>>();
        if expected != actual || outputs.len() != expected.len() {
            return Err(json!({
                "code":"result_contract_invalid",
                "message":"结果名称集合与冻结合同不一致",
                "expected":expected,
                "actual":actual,
            }));
        }
        for result in &locked.results {
            let output = outputs
                .iter()
                .find(|output| output.name == result.name)
                .expect("result name set checked");
            let contract = self.snapshot.schemas.resolve(&result.schema_ref).map_err(
                |error| json!({"code":"result_contract_invalid","message":error.to_string()}),
            )?;
            validate_schema(&output.metadata, &contract.fields).map_err(|message| {
                json!({
                    "code":"result_contract_invalid",
                    "message":message,
                    "resultName":result.name,
                    "schemaRef":result.schema_ref,
                })
            })?;
            validate_non_null_values(&output.path, &contract.fields).map_err(|message| {
                json!({
                    "code":"result_contract_invalid",
                    "message":message,
                    "resultName":result.name,
                    "schemaRef":result.schema_ref,
                })
            })?;
            if !output.metadata.snappy {
                return Err(json!({
                    "code":"result_contract_invalid",
                    "message":"结果必须使用 Snappy Parquet",
                    "resultName":result.name,
                }));
            }
        }
        Ok(())
    }

    async fn publish_results(
        &self,
        user: &CurrentUser,
        workflow_id: i32,
        attempt_id: i32,
        outputs: Vec<crate::adapters::EngineDatasetOutput>,
    ) -> Result<(), ResultCommitFailure> {
        let mut staged_artifacts = Vec::new();
        let mut drafts = Vec::new();
        let mut datasets = Vec::new();
        let roots = output_roots(&outputs);
        let _cleanup = OutputTreeCleanup(roots);
        for output in outputs {
            let staged = self
                .artifacts
                .store()
                .stage(
                    std::fs::File::open(&output.path).map_err(|error| ResultCommitFailure {
                        code: "result_publish_failed",
                        message: format!("结果发布失败: {}", error.kind()),
                    })?,
                    StageLimits {
                        max_bytes: self.result_max_file_bytes,
                    },
                )
                .map_err(|error| ResultCommitFailure {
                    code: "result_publish_failed",
                    message: format!("结果发布失败: {}", error.code()),
                })?;
            let size_bytes = i64::try_from(staged.size_bytes).map_err(|_| ResultCommitFailure {
                code: "result_publish_failed",
                message: "结果 artifact 大小溢出".into(),
            })?;
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
                super::datasets::schema_json_and_hash(&output.metadata);
            datasets.push(PendingRunDataset {
                result_name: output.name,
                schema_json,
                schema_hash,
                row_count: output.metadata.row_count as i64,
                size_bytes: output.metadata.size_bytes as i64,
                created_at_ms: now_ms(),
            });
        }
        let store = self.artifacts.store().clone();
        let _publication_guard = self.artifacts.exclusive_publication_guard().await;
        let publish_store = store.clone();
        let published_new = Arc::new(Mutex::new(Vec::<String>::new()));
        let published_by_callback = published_new.clone();
        let result = self
            .repo
            .finalize_with_artifact_outputs(
                &user.id,
                user.storage_scope_id,
                workflow_id,
                attempt_id,
                drafts,
                datasets,
                move |index, path| {
                    let existed = publish_store
                        .resolve(path)
                        .map_err(|error| {
                            crate::db::repositories::RepoError::Storage(error.to_string())
                        })?
                        .is_file();
                    publish_store
                        .publish(&mut staged_artifacts[index], path)
                        .map_err(|error| {
                            crate::db::repositories::RepoError::Storage(error.to_string())
                        })?;
                    if !existed {
                        published_by_callback
                            .lock()
                            .expect("published result paths")
                            .push(path.to_string());
                    }
                    Ok(())
                },
                now_ms(),
            )
            .await;
        if result.is_err() {
            for path in published_new.lock().expect("published result paths").iter() {
                let _ = store.remove(path);
            }
        }
        result.map(|_| ()).map_err(|error| ResultCommitFailure {
            code: if matches!(
                error,
                crate::db::repositories::ManagedWorkflowRepoError::Repository(
                    crate::db::repositories::RepoError::QuotaExceeded
                )
            ) {
                "storage_quota_exceeded"
            } else {
                "result_publish_failed"
            },
            message: if matches!(
                error,
                crate::db::repositories::ManagedWorkflowRepoError::Repository(
                    crate::db::repositories::RepoError::QuotaExceeded
                )
            ) {
                "存储配额不足".into()
            } else {
                "结果发布失败".into()
            },
        })
    }

    async fn finish_failure(
        &self,
        user: &CurrentUser,
        workflow_id: i32,
        attempt_id: Option<i32>,
        code: &str,
        message: &str,
    ) -> Result<(), String> {
        let error = json!({"code":code,"message":message});
        if let Some(attempt_id) = attempt_id {
            self.repo
                .finalize_with_outputs(
                    &user.id,
                    workflow_id,
                    attempt_id,
                    false,
                    Some(error.to_string()),
                    Vec::new(),
                    now_ms(),
                )
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        } else {
            self.repo
                .fail_without_candidate(&user.id, workflow_id, error.to_string(), now_ms())
                .await
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
    }
}

enum ExecutionFailure {
    Engine(AdapterError),
    Cancelled,
    Local(&'static str, String),
}

fn cancelled(cancel: &watch::Receiver<bool>, aggregate: &ManagedWorkflowAggregate) -> bool {
    *cancel.borrow() || aggregate.workflow.status == "cancelled_by_user"
}

fn exit_code(error: &AdapterError) -> Option<i32> {
    error
        .diagnostics
        .as_deref()
        .and_then(|value| value.get("exitCode"))
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
}

fn repairable_engine_error(code: &str, exit_code: Option<i32>) -> bool {
    if matches!(
        code,
        "engine_timeout"
            | "result_monitor_failed"
            | "result_path_escape"
            | "result_symlink_rejected"
            | "result_dir_invalid"
            | "storage_quota_exceeded"
            | "input_artifact_missing"
            | "engine_unavailable"
    ) {
        return false;
    }
    exit_code.is_some()
        || matches!(
            code,
            "result_contract_invalid"
                | "result_file_limit"
                | "result_file_too_large"
                | "result_total_too_large"
                | "result_empty"
                | "result_format_invalid"
                | "result_compression_invalid"
        )
}

fn public_engine_diagnostic(error: &AdapterError, sanitized: &SanitizedDiagnostic) -> Value {
    json!({
        "code":error.code,
        "category":error.category.as_str(),
        "message":"仿真引擎执行失败，请查看已脱敏诊断",
        "exitCode":exit_code(error),
        "stderr":sanitized,
    })
}

pub(crate) fn validate_schema(
    metadata: &ParquetMetadata,
    expected: &[SchemaFieldContract],
) -> Result<(), String> {
    if metadata.columns.len() != expected.len() {
        return Err(format!(
            "结果列数不一致: expected {}, actual {}",
            expected.len(),
            metadata.columns.len()
        ));
    }
    for (actual, expected) in metadata.columns.iter().zip(expected) {
        let expected_dtype = canonical_arrow(&expected.dtype);
        let expected_unit =
            (expected.unit.0 != "not_applicable").then_some(expected.unit.0.as_str());
        let unit_mismatch = actual
            .unit
            .as_deref()
            .is_some_and(|actual| Some(actual) != expected_unit);
        if actual.name != expected.name
            || !dtype_matches(&actual.dtype, &expected.dtype)
            || unit_mismatch
        {
            return Err(format!(
                "结果列 {} 不符合合同: expected {}/{}/{:?}, actual {}/{}/{:?}",
                expected.name,
                expected_dtype,
                expected.nullable,
                expected_unit,
                actual.dtype,
                actual.nullable,
                actual.unit
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_non_null_values(
    path: &std::path::Path,
    expected: &[SchemaFieldContract],
) -> Result<(), String> {
    let columns = expected
        .iter()
        .filter(|field| !field.nullable)
        .map(|field| field.name.clone())
        .collect::<Vec<_>>();
    match crate::storage::ParquetInspector::first_column_with_nulls(path, &columns) {
        Ok(Some(name)) => Err(format!("结果列 {name} 含 null，不符合非空合同")),
        Ok(None) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn canonical_arrow(dtype: &DataTypeContract) -> String {
    match dtype {
        DataTypeContract::Primitive { name } => match name.as_str() {
            "str" => "Utf8".into(),
            "bool" => "Boolean".into(),
            "i32" => "Int32".into(),
            "i64" => "Int64".into(),
            "u32" => "UInt32".into(),
            "u64" => "UInt64".into(),
            "f32" => "Float32".into(),
            "f64" => "Float64".into(),
            other => other.into(),
        },
        DataTypeContract::Semantic { name } => match name.as_str() {
            "u32_semantics" => "UInt32".into(),
            other => other.into(),
        },
        DataTypeContract::Constructor { name, element } => {
            format!("{}<{}>", title_case(name), canonical_arrow(element))
        }
    }
}

fn dtype_matches(actual: &str, expected: &DataTypeContract) -> bool {
    match expected {
        DataTypeContract::Primitive { name } if name == "str" => {
            matches!(actual, "Utf8" | "LargeUtf8" | "Utf8View")
        }
        DataTypeContract::Constructor { name, element }
            if matches!(name.as_str(), "list" | "matrix") =>
        {
            let element = canonical_arrow(element);
            ["List", "LargeList", "ListView", "LargeListView"]
                .iter()
                .any(|kind| {
                    actual == format!("{kind}({element})")
                        || actual.starts_with(&format!("{kind}({element},"))
                        || actual.starts_with(&format!("{kind}(non-null {element}"))
                })
        }
        _ => actual == canonical_arrow(expected),
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

fn output_roots(outputs: &[crate::adapters::EngineDatasetOutput]) -> BTreeSet<PathBuf> {
    outputs
        .iter()
        .filter_map(|output| output.path.parent().and_then(std::path::Path::parent))
        .map(std::path::Path::to_path_buf)
        .collect()
}

fn cleanup_outputs(outputs: &[crate::adapters::EngineDatasetOutput]) {
    cleanup_roots(&output_roots(outputs));
}

fn cleanup_roots(roots: &BTreeSet<PathBuf>) {
    for root in roots {
        let _ = std::fs::remove_dir_all(root);
    }
}

struct PrivateTree {
    root: PathBuf,
    keep: bool,
}

struct ResultCommitFailure {
    code: &'static str,
    message: String,
}

struct CandidateStoreFailure {
    code: &'static str,
    message: String,
}

struct OutputTreeCleanup(BTreeSet<PathBuf>);

impl Drop for OutputTreeCleanup {
    fn drop(&mut self) {
        cleanup_roots(&self.0);
    }
}

impl PrivateTree {
    fn new(root: PathBuf) -> Self {
        Self { root, keep: false }
    }

    fn keep(&mut self) {
        self.keep = true;
    }
}

impl Drop for PrivateTree {
    fn drop(&mut self) {
        if !self.keep {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{dtype_matches, validate_schema};
    use crate::catalog::{DataTypeContract, SchemaFieldContract, UnitContract};
    use crate::storage::{ParquetColumn, ParquetMetadata};

    fn constructor(name: &str, element: &str) -> DataTypeContract {
        DataTypeContract::Constructor {
            name: name.into(),
            element: Box::new(DataTypeContract::Primitive {
                name: element.into(),
            }),
        }
    }

    #[test]
    fn accepts_arrow_list_display_for_list_and_matrix_contracts() {
        let arrow = "List(Float64, field: 'element')";
        assert!(dtype_matches(arrow, &constructor("list", "f64")));
        assert!(dtype_matches(arrow, &constructor("matrix", "f64")));
    }

    #[test]
    fn accepts_large_list_and_rejects_wrong_element_types() {
        let expected = constructor("list", "f64");
        assert!(dtype_matches(
            "LargeList(Float64, field: 'element')",
            &expected
        ));
        assert!(!dtype_matches("List(Utf8, field: 'element')", &expected));
    }

    #[test]
    fn accepts_arrow_string_storage_variants_for_string_contracts() {
        let expected = DataTypeContract::Primitive { name: "str".into() };
        assert!(dtype_matches("Utf8", &expected));
        assert!(dtype_matches("LargeUtf8", &expected));
        assert!(dtype_matches("Utf8View", &expected));
        assert!(!dtype_matches("BinaryView", &expected));
    }

    #[test]
    fn logical_schema_accepts_physical_nullability_and_missing_unit_metadata() {
        let metadata = ParquetMetadata {
            columns: vec![ParquetColumn {
                name: "value".into(),
                dtype: "LargeList(Float64, field: 'element')".into(),
                nullable: true,
                unit: None,
            }],
            row_count: 1,
            size_bytes: 1,
            snappy: true,
        };
        let expected = vec![SchemaFieldContract {
            name: "value".into(),
            dtype: constructor("list", "f64"),
            required: true,
            nullable: false,
            unit: UnitContract("V".into()),
            card: None,
        }];
        assert!(validate_schema(&metadata, &expected).is_ok());

        let mut conflicting = metadata;
        conflicting.columns[0].unit = Some("A".into());
        assert!(validate_schema(&conflicting, &expected).is_err());
    }
}
