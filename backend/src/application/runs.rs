use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::adapters::{
    ENGINE_CONTRACT_VERSION, ErrorCategory, NamedInput, RunMode, RunSpec, SimEngine,
};
use crate::auth::CurrentUser;
use crate::db::models::SimulationRun;
use crate::db::repositories::{
    CreateRun, NewRunDataset, RepoError, RunDataset, RunRepo, RunRepoError, ScriptRepo,
};
use crate::storage::{ParquetInspector, StageLimits};

use super::{
    ArtifactError, ArtifactMetadata, ArtifactService, EventBus, EventWriter, MessageRepo,
    OperationKind, OperationRef, ResultOutputRef, RunScheduler, Stage, StageStatus, now_ms,
};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateRunRequest {
    pub model_version_id: String,
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

#[derive(Debug)]
pub enum RunError {
    NotFound,
    ActiveModelChanged,
    ScriptStaticInvalid,
    ScriptIncompatible(Vec<String>),
    IdempotencyConflict,
    QueueFull,
    Invalid(String),
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
            Self::IdempotencyConflict => write!(f, "clientRunId 已用于不同运行"),
            Self::QueueFull => write!(f, "运行队列已满"),
            Self::Invalid(v) => write!(f, "{v}"),
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
            RunRepoError::IdempotencyConflict => Self::IdempotencyConflict,
        }
    }
}

#[derive(Clone)]
pub struct RunService {
    repo: RunRepo,
    scripts: ScriptRepo,
    artifacts: ArtifactService,
    engine: Arc<dyn SimEngine>,
    scheduler: RunScheduler,
    messages: MessageRepo,
    events: EventBus,
    data_dir: PathBuf,
}

pub(crate) struct RunServiceDependencies {
    pub repo: RunRepo,
    pub scripts: ScriptRepo,
    pub artifacts: ArtifactService,
    pub engine: Arc<dyn SimEngine>,
    pub scheduler: RunScheduler,
    pub messages: MessageRepo,
    pub events: EventBus,
    pub data_dir: PathBuf,
}

impl RunService {
    pub(crate) fn new(dependencies: RunServiceDependencies) -> Self {
        let RunServiceDependencies {
            repo,
            scripts,
            artifacts,
            engine,
            scheduler,
            messages,
            events,
            data_dir,
        } = dependencies;
        Self {
            repo,
            scripts,
            artifacts,
            engine,
            scheduler,
            messages,
            events,
            data_dir,
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
        let version_id = request
            .model_version_id
            .parse::<i32>()
            .map_err(|_| RunError::Invalid("modelVersionId 无效".into()))?;
        let script = self.repo_for_script(user, script_id).await?;
        let ticket = self
            .scheduler
            .try_admit(&user.id, script.session_id)
            .map_err(|_| RunError::QueueFull)?;
        let created = self
            .repo
            .create(
                &user.id,
                CreateRun {
                    script_id,
                    model_version_id: version_id,
                    client_run_id: request.client_run_id,
                    engine_contract_version: ENGINE_CONTRACT_VERSION.into(),
                    created_at_ms: now_ms(),
                },
            )
            .await?;
        if !created.replayed {
            let service = self.clone();
            let principal = user.clone();
            let run_id = created.run.id;
            tokio::spawn(async move {
                ticket.run(service.execute(principal, run_id)).await;
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
        let script = self.scripts.find(&user.id, run.script_resource_id).await?;
        Ok(RunSummary {
            run,
            script_name: script.name,
        })
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
                let script_name = script_names
                    .get(&run.script_resource_id)
                    .cloned()
                    .ok_or(RunError::NotFound)?;
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
    async fn execute(&self, user: CurrentUser, id: i32) {
        if let Err(error) = self.execute_inner(&user, id).await {
            let run = self.repo.find(&user.id, id).await.ok();
            let failed_stage = run
                .as_ref()
                .map(|run| run.stage.clone())
                .unwrap_or_else(|| "run".to_string());
            eprintln!("[run] failed id={id}, stage={failed_stage}: {error}");
            let reason = public_failure_reason(&error);
            let failed = self
                .repo
                .fail(
                    &user.id,
                    id,
                    &failed_stage,
                    serde_json::json!({"code":"run_failed","reason":reason}).to_string(),
                    now_ms(),
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
            }
        }
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
        let root = self.data_dir.join("runs").join(id.to_string());
        let input = root.join("input");
        let result = root.join("result");
        std::fs::create_dir_all(&input).map_err(RunError::Io)?;
        std::fs::create_dir_all(&result).map_err(RunError::Io)?;
        let script_artifact = self
            .artifacts
            .find_owned(user.storage_scope_id, context.run.script_artifact_id)
            .await?;
        let script_source = self
            .artifacts
            .store()
            .resolve(&script_artifact.storage_path)
            .map_err(ArtifactError::from)?;
        let script_path = PathBuf::from("script.rss");
        std::fs::copy(script_source, input.join(&script_path)).map_err(RunError::Io)?;
        let mut named = Vec::new();
        for frame in context.frames {
            let artifact = self
                .artifacts
                .find_owned(user.storage_scope_id, frame.artifact_id)
                .await?;
            let source = self
                .artifacts
                .store()
                .resolve(&artifact.storage_path)
                .map_err(ArtifactError::from)?;
            let relative = PathBuf::from("model").join(format!("{}.parquet", frame.table_name));
            if let Some(parent) = input.join(&relative).parent() {
                std::fs::create_dir_all(parent).map_err(RunError::Io)?
            }
            std::fs::copy(source, input.join(&relative)).map_err(RunError::Io)?;
            named.push(NamedInput {
                alias: frame.table_name,
                relative_path: relative,
            });
        }
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
                input_tree: input,
                script_path,
                named_inputs: named,
                result_dir: result,
                timeout: std::time::Duration::from_secs(300),
            })
            .await
            .map_err(RunError::Adapter)?;
        let mut committed = Vec::new();
        let mut datasets = Vec::new();
        for output in output.datasets {
            let metadata = ParquetInspector::metadata(&output.path)
                .map_err(|error| RunError::Invalid(error.to_string()))?;
            if !metadata.snappy {
                return Err(RunError::Invalid("运行结果必须是 Snappy Parquet".into()));
            }
            let staged = self
                .artifacts
                .store()
                .stage(
                    std::fs::File::open(&output.path).map_err(RunError::Io)?,
                    StageLimits {
                        max_bytes: 500 * 1024 * 1024,
                    },
                )
                .map_err(ArtifactError::from)?;
            let artifact = match self
                .artifacts
                .commit(user.storage_scope_id, staged, ArtifactMetadata::parquet())
                .await
            {
                Ok(v) => v,
                Err(error) => {
                    for artifact_id in committed {
                        let _ = self
                            .artifacts
                            .release(user.storage_scope_id, artifact_id)
                            .await;
                    }
                    return Err(error.into());
                }
            };
            committed.push(artifact.id);
            let (schema_json, schema_hash) =
                crate::application::datasets::schema_json_and_hash(&metadata);
            datasets.push(NewRunDataset {
                artifact_id: artifact.id,
                result_name: output.name,
                schema_json,
                schema_hash,
                row_count: metadata.row_count as i64,
                size_bytes: metadata.size_bytes as i64,
                created_at_ms: now_ms(),
            });
        }
        match self.repo.complete(&user.id, id, datasets, now_ms()).await {
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
                let _ = std::fs::remove_dir_all(root);
                Ok(())
            }
            Err(error) => {
                for artifact_id in committed {
                    let _ = self
                        .artifacts
                        .release(user.storage_scope_id, artifact_id)
                        .await;
                }
                Err(error.into())
            }
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
        RunError::Invalid(_) => "result",
        RunError::Artifact(_) | RunError::Io(_) => "io",
        _ => "internal",
    }
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
