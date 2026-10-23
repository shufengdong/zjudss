use serde_json::{Value, json};
use std::{
    collections::HashSet,
    io::Cursor,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use crate::adapters::{
    ENGINE_CONTRACT_VERSION, InspectSpec, NamedInput, RunMode, RunSpec, SimEngine,
};
use crate::agent::knowledge::AgentKnowledge;
use crate::application::{
    ArtifactMetadata, ArtifactService, EventBus, EventWriter, OperationKind, OperationRef,
    ResultColumn, ResultOutputRef, Stage, StageStatus, WorkspaceService, now_ms,
};
use crate::auth::Principal;
use crate::db::models::{
    DataframeBinding, NewResultDataset, NewSimulationTask, ResultDataset, SimulationTask,
    VersionArtifact,
};
use crate::db::repositories::{MessageRepo, SimulationRepo};
use crate::ingestion::normalize_csv_to_parquet;
use crate::storage::{ParquetInspector, StageLimits};

#[derive(Debug, Clone)]
pub struct RunStudyRequest {
    pub principal: Principal,
    pub session_id: i32,
    pub model_version_id: i32,
    pub study_type: String,
    pub study_spec: Value,
    pub script: Option<String>,
    pub skill_refs: Vec<Value>,
}
#[derive(Debug, Clone)]
pub struct SimulationOutcome {
    pub task: SimulationTask,
    pub datasets: Vec<ResultDataset>,
}
#[derive(Debug)]
pub struct SimulationError {
    pub code: &'static str,
    pub message: String,
    pub task_id: Option<i32>,
}
impl SimulationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            task_id: None,
        }
    }
    fn task(mut self, id: i32) -> Self {
        self.task_id = Some(id);
        self
    }
}
impl std::fmt::Display for SimulationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for SimulationError {}

#[derive(Clone)]
pub struct SimulationService {
    repo: SimulationRepo,
    workspaces: WorkspaceService,
    artifacts: ArtifactService,
    messages: MessageRepo,
    events: EventBus,
    engine: Arc<dyn SimEngine>,
    knowledge: Arc<AgentKnowledge>,
    data_dir: PathBuf,
}

impl SimulationService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: SimulationRepo,
        workspaces: WorkspaceService,
        artifacts: ArtifactService,
        messages: MessageRepo,
        events: EventBus,
        engine: Arc<dyn SimEngine>,
        knowledge: Arc<AgentKnowledge>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            repo,
            workspaces,
            artifacts,
            messages,
            events,
            engine,
            knowledge,
            data_dir,
        }
    }

    pub async fn run(
        &self,
        request: RunStudyRequest,
    ) -> Result<SimulationOutcome, SimulationError> {
        let version = self
            .workspaces
            .find_version(&request.principal.access, request.model_version_id)
            .await
            .map_err(|e| SimulationError::new("model_version_not_found", e.to_string()))?;
        if version.status != "ready" {
            return Err(SimulationError::new(
                "model_version_not_ready",
                "只能运行 ready 模型版本",
            ));
        }
        let workspace = self
            .workspaces
            .get(&request.principal.access, request.session_id)
            .await
            .map_err(|e| SimulationError::new("workspace_not_found", e.to_string()))?;
        if workspace.workspace.id != version.workspace_id {
            return Err(SimulationError::new(
                "model_version_not_found",
                "资源不存在",
            ));
        }
        let inputs = self
            .workspaces
            .inputs(&request.principal.access, version.id)
            .await
            .map_err(|e| SimulationError::new("model_inputs_unavailable", e.to_string()))?;
        let scope = self
            .artifacts
            .resolve_scope(&request.principal.storage)
            .await
            .map_err(|e| SimulationError::new("storage_unavailable", e.to_string()))?;
        let entry = inputs
            .iter()
            .find(|(input, _)| input.role == "entry")
            .map(|(input, _)| input.clone());
        let script_artifact_id = if matches!(version.source_kind.as_str(), "rss" | "rssbundle") {
            entry
                .clone()
                .ok_or_else(|| SimulationError::new("model_entry_missing", "RSS 模型缺少入口"))?
                .artifact_id
        } else {
            let script = request.script.as_deref().ok_or_else(|| {
                SimulationError::new("study_script_required", "本次仿真缺少实际执行脚本")
            })?;
            let staged = self
                .artifacts
                .store()
                .stage(
                    Cursor::new(script.as_bytes()),
                    StageLimits {
                        max_bytes: 2 * 1024 * 1024,
                    },
                )
                .map_err(|e| SimulationError::new("script_stage_failed", e.to_string()))?;
            self.artifacts
                .commit(
                    scope.id,
                    staged,
                    ArtifactMetadata {
                        format: "rss".into(),
                        media_type: Some("text/plain".into()),
                    },
                )
                .await
                .map_err(|e| SimulationError::new("script_commit_failed", e.to_string()))?
                .id
        };
        let task = self
            .repo
            .create(
                &request.principal.access,
                NewSimulationTask {
                    session_id: request.session_id,
                    status: "pending".into(),
                    stage: Some("prepare".into()),
                    study_type: request.study_type.clone(),
                    model_version_id: version.id,
                    study_spec_json: crate::application::canonical_json(&request.study_spec),
                    script_artifact_id,
                    agent_bundle_version: self.knowledge.manifest.agent_bundle_version.clone(),
                    agent_skill_refs_json: crate::application::canonical_json(&Value::Array(
                        request.skill_refs.clone(),
                    )),
                    engine_contract_version: ENGINE_CONTRACT_VERSION.into(),
                    exit_code: None,
                    error_json: None,
                    created_at_ms: now_ms(),
                    started_at_ms: None,
                    finished_at_ms: None,
                },
            )
            .await
            .map_err(|e| SimulationError::new("task_create_failed", e.to_string()))?;
        let root = self.data_dir.join("task-work").join(task.id.to_string());
        let outcome = self
            .run_task(
                &request,
                &version.source_kind,
                inputs,
                entry,
                script_artifact_id,
                scope.id,
                &task,
                &root,
            )
            .await;
        let _ = std::fs::remove_dir_all(&root);
        match outcome {
            Ok(datasets) => {
                let task = self
                    .repo
                    .find(&request.principal.access, task.id)
                    .await
                    .map_err(|e| {
                        SimulationError::new("task_read_failed", e.to_string()).task(task.id)
                    })?;
                Ok(SimulationOutcome { task, datasets })
            }
            Err(error) => {
                let stable = json!({"code": error.code, "message": error.message}).to_string();
                let _ = self
                    .repo
                    .fail(
                        &request.principal.access,
                        task.id,
                        "collect",
                        stable,
                        now_ms(),
                    )
                    .await;
                let writer = EventWriter::new(
                    request.principal.access.clone(),
                    request.session_id,
                    self.messages.clone(),
                    self.events.clone(),
                );
                let _ = writer
                    .progress(
                        OperationRef {
                            kind: OperationKind::Simulation,
                            id: task.id.to_string(),
                        },
                        stages(Some("collect"), true),
                    )
                    .await;
                Err(error.task(task.id))
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn run_task(
        &self,
        request: &RunStudyRequest,
        source_kind: &str,
        inputs: Vec<(VersionArtifact, Option<DataframeBinding>)>,
        entry: Option<VersionArtifact>,
        script_artifact_id: i32,
        scope_id: i32,
        task: &SimulationTask,
        root: &Path,
    ) -> Result<Vec<ResultDataset>, SimulationError> {
        let input_tree = root.join("input");
        let result_dir = root.join("results-staging");
        std::fs::create_dir_all(&input_tree)
            .and_then(|_| std::fs::create_dir_all(&result_dir))
            .map_err(|e| SimulationError::new("task_staging_failed", e.to_string()))?;
        let writer = EventWriter::new(
            request.principal.access.clone(),
            request.session_id,
            self.messages.clone(),
            self.events.clone(),
        );
        self.stage(&request.principal, task.id, "prepare", &writer)
            .await?;
        let named_inputs = if source_kind == "dataframes" {
            self.materialize_dataframes(scope_id, &inputs, &input_tree)
                .await?
        } else {
            self.materialize_tree(scope_id, source_kind, &inputs, &input_tree)
                .await?;
            Vec::new()
        };
        let script_path = match source_kind {
            "rssbundle" => PathBuf::from("model.rssbundle"),
            "rss" => entry
                .as_ref()
                .and_then(|input| input.relative_path.as_deref())
                .map(PathBuf::from)
                .ok_or_else(|| {
                    SimulationError::new("model_entry_missing", "RSS 模型入口路径缺失")
                })?,
            _ => {
                self.materialize_script(scope_id, script_artifact_id, &input_tree)
                    .await?
            }
        };
        self.stage(&request.principal, task.id, "inspect", &writer)
            .await?;
        if source_kind == "rss" {
            let inspect = entry
                .as_ref()
                .and_then(|v| v.relative_path.as_deref())
                .map(PathBuf::from)
                .unwrap_or_else(|| script_path.clone());
            self.engine
                .inspect(InspectSpec {
                    input_tree: input_tree.clone(),
                    model_path: inspect,
                    timeout: Duration::from_secs(60),
                })
                .await
                .map_err(|e| SimulationError::new("engine_inspect_failed", e.to_string()))?;
        }
        self.stage(&request.principal, task.id, "sim", &writer)
            .await?;
        let mode = match source_kind {
            "rssbundle" => RunMode::Bundle,
            "dataframes" => RunMode::Dataframe,
            "rss" => RunMode::Rss,
            _ => {
                return Err(SimulationError::new(
                    "model_source_unsupported",
                    source_kind,
                ));
            }
        };
        let spec = RunSpec {
            mode,
            input_tree: input_tree.clone(),
            script_path,
            named_inputs,
            result_dir: result_dir.clone(),
            timeout: Duration::from_secs(600),
        };
        let output = if mode == RunMode::Bundle {
            self.engine.run_bundle(spec).await
        } else {
            self.engine.run_dataframe(spec).await
        }
        .map_err(|e| SimulationError::new("engine_run_failed", e.to_string()))?;
        self.stage(&request.principal, task.id, "collect", &writer)
            .await?;
        let (drafts, committed) = self.collect(scope_id, task.id, output).await?;
        let datasets = match self
            .repo
            .complete(&request.principal.access, task.id, drafts, now_ms())
            .await
        {
            Ok(v) => v,
            Err(e) => {
                for id in committed {
                    let _ = self.artifacts.release(scope_id, id).await;
                }
                return Err(SimulationError::new("result_publish_failed", e.to_string()));
            }
        };
        let outputs = datasets.iter().map(output_ref).collect();
        writer
            .result(
                Some(task.id),
                None,
                &format!("{} 仿真结果", request.study_type),
                outputs,
            )
            .await
            .map_err(|e| SimulationError::new("result_event_failed", e.to_string()))?;
        writer
            .progress(
                OperationRef {
                    kind: OperationKind::Simulation,
                    id: task.id.to_string(),
                },
                stages(None, false),
            )
            .await
            .map_err(|e| SimulationError::new("progress_event_failed", e.to_string()))?;
        Ok(datasets)
    }

    async fn materialize_dataframes(
        &self,
        scope_id: i32,
        inputs: &[(VersionArtifact, Option<DataframeBinding>)],
        tree: &Path,
    ) -> Result<Vec<NamedInput>, SimulationError> {
        let mut aliases = HashSet::new();
        let mut named = Vec::new();
        for (input, binding) in inputs {
            let binding = binding.as_ref().ok_or_else(|| {
                SimulationError::new(
                    "dataframe_binding_missing",
                    format!("input {} 未映射", input.id),
                )
            })?;
            let alias = binding
                .table_kind
                .clone()
                .filter(|v| valid_alias(v))
                .ok_or_else(|| {
                    SimulationError::new(
                        "dataframe_binding_invalid",
                        format!("input {} 映射无效", input.id),
                    )
                })?;
            if !aliases.insert(alias.clone()) {
                return Err(SimulationError::new("dataframe_binding_duplicate", alias));
            }
            let artifact = self
                .artifacts
                .find_owned(scope_id, input.artifact_id)
                .await
                .map_err(|e| SimulationError::new("model_input_missing", e.to_string()))?;
            let source = self
                .artifacts
                .store()
                .resolve(&artifact.storage_path)
                .map_err(|e| SimulationError::new("model_input_missing", e.to_string()))?;
            let relative_path = PathBuf::from(format!("inputs/{}.parquet", input.id));
            let destination = tree.join(&relative_path);
            std::fs::create_dir_all(destination.parent().unwrap())
                .map_err(|e| SimulationError::new("task_staging_failed", e.to_string()))?;
            if artifact.format.eq_ignore_ascii_case("csv") {
                normalize_csv_to_parquet(&source, &destination)
                    .map_err(|e| SimulationError::new("input_normalize_failed", e.to_string()))?;
            } else {
                std::fs::copy(source, &destination)
                    .map_err(|e| SimulationError::new("task_staging_failed", e.to_string()))?;
            }
            named.push(NamedInput {
                alias,
                relative_path,
            });
        }
        Ok(named)
    }

    async fn materialize_tree(
        &self,
        scope_id: i32,
        source_kind: &str,
        inputs: &[(VersionArtifact, Option<DataframeBinding>)],
        tree: &Path,
    ) -> Result<(), SimulationError> {
        for (input, _) in inputs {
            let artifact = self
                .artifacts
                .find_owned(scope_id, input.artifact_id)
                .await
                .map_err(|e| SimulationError::new("model_input_missing", e.to_string()))?;
            let source = self
                .artifacts
                .store()
                .resolve(&artifact.storage_path)
                .map_err(|e| SimulationError::new("model_input_missing", e.to_string()))?;
            let raw = safe_relative(input.relative_path.as_deref().unwrap_or("input"))?;
            let relative = if source_kind == "rssbundle" {
                PathBuf::from("model.rssbundle").join(raw)
            } else {
                raw
            };
            let destination = tree.join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| SimulationError::new("task_staging_failed", e.to_string()))?;
            }
            std::fs::copy(source, destination)
                .map_err(|e| SimulationError::new("task_staging_failed", e.to_string()))?;
        }
        Ok(())
    }

    async fn materialize_script(
        &self,
        scope_id: i32,
        id: i32,
        tree: &Path,
    ) -> Result<PathBuf, SimulationError> {
        let artifact = self
            .artifacts
            .find_owned(scope_id, id)
            .await
            .map_err(|e| SimulationError::new("script_missing", e.to_string()))?;
        let source = self
            .artifacts
            .store()
            .resolve(&artifact.storage_path)
            .map_err(|e| SimulationError::new("script_missing", e.to_string()))?;
        let relative = PathBuf::from("generated/study.rss");
        let destination = tree.join(&relative);
        std::fs::create_dir_all(destination.parent().unwrap())
            .map_err(|e| SimulationError::new("task_staging_failed", e.to_string()))?;
        std::fs::copy(source, destination)
            .map_err(|e| SimulationError::new("task_staging_failed", e.to_string()))?;
        Ok(relative)
    }

    async fn collect(
        &self,
        scope_id: i32,
        task_id: i32,
        output: crate::adapters::EngineRunOutput,
    ) -> Result<(Vec<NewResultDataset>, Vec<i32>), SimulationError> {
        let mut committed = Vec::new();
        let mut drafts = Vec::new();
        for output in output.datasets {
            let metadata = ParquetInspector::metadata(&output.path)
                .map_err(|e| SimulationError::new("result_parquet_invalid", e.to_string()))?;
            if !metadata.snappy {
                return Err(SimulationError::new(
                    "result_compression_invalid",
                    "结果必须为 Snappy Parquet",
                ));
            }
            let file = std::fs::File::open(&output.path)
                .map_err(|e| SimulationError::new("result_read_failed", e.to_string()))?;
            let staged = self
                .artifacts
                .store()
                .stage(
                    file,
                    StageLimits {
                        max_bytes: 500 * 1024 * 1024,
                    },
                )
                .map_err(|e| SimulationError::new("result_stage_failed", e.to_string()))?;
            let artifact = match self
                .artifacts
                .commit(
                    scope_id,
                    staged,
                    ArtifactMetadata {
                        format: "parquet".into(),
                        media_type: Some("application/vnd.apache.parquet".into()),
                    },
                )
                .await
            {
                Ok(v) => v,
                Err(e) => {
                    for id in committed {
                        let _ = self.artifacts.release(scope_id, id).await;
                    }
                    return Err(SimulationError::new("result_commit_failed", e.to_string()));
                }
            };
            committed.push(artifact.id);
            let (schema_json, schema_hash) =
                crate::application::datasets::schema_json_and_hash(&metadata);
            drafts.push(NewResultDataset {
                task_id: Some(task_id),
                analysis_run_id: None,
                artifact_id: artifact.id,
                result_name: output.name,
                schema_json,
                schema_hash,
                row_count: i64::try_from(metadata.row_count)
                    .map_err(|_| SimulationError::new("result_too_large", "row count 超限"))?,
                size_bytes: i64::try_from(metadata.size_bytes)
                    .map_err(|_| SimulationError::new("result_too_large", "结果大小超限"))?,
                created_at_ms: now_ms(),
            });
        }
        Ok((drafts, committed))
    }

    async fn stage(
        &self,
        principal: &Principal,
        id: i32,
        name: &str,
        writer: &EventWriter,
    ) -> Result<(), SimulationError> {
        self.repo
            .set_stage(&principal.access, id, name, now_ms())
            .await
            .map_err(|e| SimulationError::new("task_update_failed", e.to_string()))?;
        writer
            .progress(
                OperationRef {
                    kind: OperationKind::Simulation,
                    id: id.to_string(),
                },
                stages(Some(name), false),
            )
            .await
            .map_err(|e| SimulationError::new("progress_event_failed", e.to_string()))?;
        Ok(())
    }
}

fn output_ref(dataset: &ResultDataset) -> ResultOutputRef {
    let columns: Vec<crate::application::datasets::DatasetColumn> =
        serde_json::from_str(&dataset.schema_json).unwrap_or_default();
    ResultOutputRef {
        dataset_id: dataset.id.to_string(),
        name: dataset.result_name.clone(),
        row_count: dataset.row_count,
        columns: columns
            .into_iter()
            .map(|c| ResultColumn {
                name: c.name,
                dtype: c.dtype,
                nullable: c.nullable,
                unit: c.unit,
            })
            .collect(),
        preview_rows: None,
        preview_truncated: dataset.row_count > 0,
        default_view_id: None,
        views: vec![],
    }
}
fn valid_alias(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}
fn safe_relative(value: &str) -> Result<PathBuf, SimulationError> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, Component::Normal(_)))
    {
        Err(SimulationError::new("model_path_invalid", value))
    } else {
        Ok(path.to_path_buf())
    }
}
fn stages(active: Option<&str>, failed: bool) -> Vec<Stage> {
    let order = ["prepare", "inspect", "sim", "collect"];
    [
        ("prepare", "准备输入"),
        ("inspect", "检查模型"),
        ("sim", "执行仿真"),
        ("collect", "收集结果"),
    ]
    .into_iter()
    .map(|(key, label)| {
        let status = match active {
            None => StageStatus::Done,
            Some(value) if value == key && failed => StageStatus::Failed,
            Some(value) if value == key => StageStatus::Running,
            Some(value)
                if order.iter().position(|v| *v == key)
                    < order.iter().position(|v| *v == value) =>
            {
                StageStatus::Done
            }
            Some(_) => StageStatus::Pending,
        };
        Stage {
            key: key.into(),
            label: label.into(),
            status,
        }
    })
    .collect()
}
