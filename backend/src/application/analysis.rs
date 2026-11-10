//! 结果分析状态机：冻结输入、调度受控脚本、验证/发布输出并投影持久化消息。
//!
//! 输入与输出物理由子模块拥有；本门面保留阶段迁移、原子发布和失败收敛。

use std::{
    collections::HashSet,
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use serde_json::{Value, json};

use crate::adapters::{ENGINE_CONTRACT_VERSION, RunMode, RunSpec, SimEngine};
use crate::agent::knowledge::AgentKnowledge;
use crate::application::{
    ArtifactMetadata, ArtifactService, DatasetViewRef, EventBus, EventWriter, OperationKind,
    OperationRef, ResultOutputRef, Stage, StageStatus, VisualizationKind, now_ms,
};
use crate::auth::CurrentUser;
use crate::db::models::{NewResultAnalysisRun, ResultAnalysisRun};
use crate::db::repositories::{
    AnalysisRepo, DataResourceRepo, MessageRepo, ResultRepo, RunDataset,
};
use crate::storage::StageLimits;

mod inputs;
mod outputs;

use crate::application::result_sources::ResultSourceService;
use inputs::ResolvedAnalysisInputs;
use outputs::{collect_outputs, validate_output_names};

#[derive(Debug, Clone)]
pub struct AnalysisInput {
    pub source: AnalysisInputSource,
    pub alias: String,
}

#[derive(Debug, Clone)]
pub enum AnalysisInputSource {
    DatasetId(i32),
    DataResourceId(String),
}

#[derive(Debug, Clone)]
pub struct AnalysisRequest {
    pub principal: CurrentUser,
    pub session_id: i32,
    pub request_message_id: Option<i32>,
    pub intent: Value,
    pub script: String,
    pub inputs: Vec<AnalysisInput>,
    pub expected_outputs: Vec<String>,
    pub skill_refs: Vec<Value>,
}

#[derive(Debug, Clone)]
pub struct AnalysisOutcome {
    pub run: ResultAnalysisRun,
    pub datasets: Vec<RunDataset>,
}

#[derive(Debug)]
pub struct AnalysisError {
    pub code: &'static str,
    pub message: String,
    pub details: Option<Value>,
    pub run_id: Option<i32>,
}

impl AnalysisError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
            run_id: None,
        }
    }
    fn run(mut self, id: i32) -> Self {
        self.run_id = Some(id);
        self
    }
}
impl std::fmt::Display for AnalysisError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for AnalysisError {}

#[derive(Clone)]
/// 拥有分析阶段迁移、调度、原子输出发布和消息投影的应用门面。
pub struct AnalysisService {
    repo: AnalysisRepo,
    results: ResultRepo,
    artifacts: ArtifactService,
    messages: MessageRepo,
    events: EventBus,
    engine: Arc<dyn SimEngine>,
    knowledge: Arc<AgentKnowledge>,
    data_dir: PathBuf,
    data_resources: DataResourceRepo,
    sources: ResultSourceService,
}

impl AnalysisService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: AnalysisRepo,
        results: ResultRepo,
        artifacts: ArtifactService,
        messages: MessageRepo,
        events: EventBus,
        engine: Arc<dyn SimEngine>,
        knowledge: Arc<AgentKnowledge>,
        data_dir: PathBuf,
        data_resources: DataResourceRepo,
    ) -> Self {
        let sources = ResultSourceService::new(results.clone(), artifacts.clone());
        Self {
            repo,
            results,
            artifacts,
            messages,
            events,
            engine,
            knowledge,
            data_dir,
            data_resources,
            sources,
        }
    }

    pub async fn run(&self, request: AnalysisRequest) -> Result<AnalysisOutcome, AnalysisError> {
        validate_request(&request)?;
        let resolved_inputs = ResolvedAnalysisInputs::resolve(
            &request.inputs,
            &request.principal,
            request.session_id,
            &self.sources,
            &self.data_resources,
        )
        .await?;
        let scope = self
            .artifacts
            .resolve_scope(request.principal.storage_scope_id)
            .await
            .map_err(|error| AnalysisError::new("storage_unavailable", error.to_string()))?;
        let script = self
            .commit_bytes(scope.id, request.script.as_bytes(), "rss", "text/plain")
            .await
            .map_err(|error| {
                AnalysisError::new("analysis_script_commit_failed", error.to_string())
            })?;
        let run = self
            .repo
            .create(
                &request.principal.id,
                NewResultAnalysisRun {
                    session_id: request.session_id,
                    request_message_id: request.request_message_id,
                    intent_json: crate::application::canonical_json(&request.intent),
                    script_artifact_id: script.id,
                    status: "pending".into(),
                    stage: "script".into(),
                    error_json: None,
                    agent_bundle_version: self.knowledge.manifest.agent_bundle_version.clone(),
                    agent_skill_refs_json: crate::application::canonical_json(&Value::Array(
                        request.skill_refs.clone(),
                    )),
                    engine_contract_version: ENGINE_CONTRACT_VERSION.into(),
                    created_at_ms: now_ms(),
                    started_at_ms: None,
                    finished_at_ms: None,
                },
                resolved_inputs.snapshot_rows(),
            )
            .await
            .map_err(|error| AnalysisError::new("analysis_create_failed", error.to_string()))?;
        let root = self.data_dir.join("analysis-work").join(run.id.to_string());
        let result = self
            .run_inner(
                &request,
                &run,
                &resolved_inputs,
                scope.id,
                &script.relative_path,
                &root,
            )
            .await;
        let _ = std::fs::remove_dir_all(&root);
        match result {
            Ok((run, datasets)) => Ok(AnalysisOutcome { run, datasets }),
            Err(error) => {
                let _ = self
                    .repo
                    .fail(
                        &request.principal.id,
                        run.id,
                        "present",
                        json!({"code":error.code,"message":error.message}).to_string(),
                        now_ms(),
                    )
                    .await;
                Err(error.run(run.id))
            }
        }
    }

    async fn run_inner(
        &self,
        request: &AnalysisRequest,
        run: &ResultAnalysisRun,
        resolved_inputs: &ResolvedAnalysisInputs,
        scope_id: i32,
        script_artifact_path: &str,
        root: &Path,
    ) -> Result<(ResultAnalysisRun, Vec<RunDataset>), AnalysisError> {
        let tree = root.join("input");
        let output_dir = root.join("results-staging");
        std::fs::create_dir_all(tree.join("generated"))
            .and_then(|_| std::fs::create_dir_all(tree.join("inputs")))
            .and_then(|_| std::fs::create_dir_all(&output_dir))
            .map_err(|error| AnalysisError::new("analysis_staging_failed", error.to_string()))?;
        let writer = EventWriter::new(
            request.principal.id.clone(),
            request.session_id,
            self.messages.clone(),
            self.events.clone(),
        );
        self.stage(&request.principal, run.id, "script", &writer)
            .await?;
        let script_source = self
            .artifacts
            .store()
            .resolve(script_artifact_path)
            .map_err(|error| AnalysisError::new("analysis_script_missing", error.to_string()))?;
        std::fs::copy(script_source, tree.join("generated/analysis.rss"))
            .map_err(|error| AnalysisError::new("analysis_staging_failed", error.to_string()))?;
        let named_inputs = resolved_inputs
            .stage(&self.artifacts, scope_id, &tree)
            .await?;
        self.stage(&request.principal, run.id, "run", &writer)
            .await?;
        let output = self
            .engine
            .run_analysis(RunSpec {
                mode: RunMode::Analysis,
                input_tree: tree,
                script_path: PathBuf::from("generated/analysis.rss"),
                named_inputs,
                result_dir: output_dir,
                timeout: Duration::from_secs(300),
            })
            .await
            .map_err(|error| AnalysisError::new("analysis_engine_failed", error.to_string()))?;
        validate_output_names(&output.datasets, &request.expected_outputs)?;
        self.stage(&request.principal, run.id, "collect", &writer)
            .await?;
        let committed = collect_outputs(output.datasets, run.id, scope_id, &self.artifacts).await?;
        self.stage(&request.principal, run.id, "present", &writer)
            .await?;
        let (run, datasets) = match self
            .repo
            .complete(&request.principal.id, run.id, committed.drafts(), now_ms())
            .await
        {
            Ok(value) => value,
            Err(error) => {
                committed.release(&self.artifacts, scope_id).await;
                return Err(AnalysisError::new(
                    "analysis_publish_failed",
                    error.to_string(),
                ));
            }
        };
        let mut outputs = Vec::new();
        for dataset in &datasets {
            outputs.push(self.result_ref(&request.principal, dataset).await?);
        }
        writer
            .result(None, Some(run.id), "结果分析", outputs)
            .await
            .map_err(|error| AnalysisError::new("analysis_event_failed", error.to_string()))?;
        writer
            .progress(
                OperationRef {
                    kind: OperationKind::Analysis,
                    id: run.id.to_string(),
                },
                analysis_stages(None, false),
            )
            .await
            .map_err(|error| AnalysisError::new("analysis_event_failed", error.to_string()))?;
        Ok((run, datasets))
    }

    pub async fn find(
        &self,
        principal: &CurrentUser,
        id: i32,
    ) -> Result<(ResultAnalysisRun, Vec<RunDataset>), AnalysisError> {
        let run = self
            .repo
            .find(&principal.id, id)
            .await
            .map_err(|error| AnalysisError::new("analysis_not_found", error.to_string()))?;
        let datasets = self
            .results
            .list_analysis(&principal.id, id)
            .await
            .map_err(|error| AnalysisError::new("analysis_not_found", error.to_string()))?;
        Ok((run, datasets))
    }

    async fn commit_bytes(
        &self,
        scope_id: i32,
        bytes: &[u8],
        format: &str,
        media_type: &str,
    ) -> Result<crate::application::ArtifactRef, crate::application::ArtifactError> {
        let staged = self
            .artifacts
            .store()
            .stage(
                Cursor::new(bytes),
                StageLimits {
                    max_bytes: 2 * 1024 * 1024,
                },
            )
            .map_err(crate::application::ArtifactError::from)?;
        self.artifacts
            .commit(
                scope_id,
                staged,
                ArtifactMetadata {
                    format: format.into(),
                    media_type: Some(media_type.into()),
                },
            )
            .await
    }

    async fn stage(
        &self,
        principal: &CurrentUser,
        id: i32,
        name: &str,
        writer: &EventWriter,
    ) -> Result<(), AnalysisError> {
        self.repo
            .set_stage(&principal.id, id, name, now_ms())
            .await
            .map_err(|error| AnalysisError::new("analysis_update_failed", error.to_string()))?;
        writer
            .progress(
                OperationRef {
                    kind: OperationKind::Analysis,
                    id: id.to_string(),
                },
                analysis_stages(Some(name), false),
            )
            .await
            .map_err(|error| AnalysisError::new("analysis_event_failed", error.to_string()))?;
        Ok(())
    }

    async fn result_ref(
        &self,
        principal: &CurrentUser,
        dataset: &RunDataset,
    ) -> Result<ResultOutputRef, AnalysisError> {
        let views = self
            .results
            .views(&principal.id, dataset.id)
            .await
            .map_err(|error| AnalysisError::new("dataset_view_read_failed", error.to_string()))?;
        let mut output = ResultOutputRef::from_dataset(dataset);
        output.views = views
            .into_iter()
            .filter_map(|view| {
                view_kind(&view.kind).map(|kind| DatasetViewRef {
                    id: view.id.to_string(),
                    name: view.name,
                    kind,
                    created_at: view.created_at_ms,
                })
            })
            .collect();
        Ok(output)
    }
}

fn validate_request(request: &AnalysisRequest) -> Result<(), AnalysisError> {
    if request.script.len() > 2 * 1024 * 1024
        || request.script.contains("http://")
        || request.script.contains("https://")
        || request.script.contains("../")
        || request.script.contains("import \"")
        || request.script.contains("include \"")
    {
        return Err(AnalysisError::new(
            "analysis_script_unsafe",
            "脚本包含远程或非绑定路径",
        ));
    }
    if request.inputs.is_empty() || request.inputs.len() > 20 {
        return Err(AnalysisError::new(
            "analysis_inputs_invalid",
            "输入数量必须为 1..=20",
        ));
    }
    let mut aliases = HashSet::new();
    for input in &request.inputs {
        let mut chars = input.alias.chars();
        if !chars
            .next()
            .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
            || !chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
            || !aliases.insert(input.alias.clone())
        {
            return Err(AnalysisError::new("analysis_alias_invalid", &input.alias));
        }
    }
    if request.expected_outputs.is_empty() || request.expected_outputs.len() > 20 {
        return Err(AnalysisError::new(
            "analysis_outputs_required",
            "必须声明 1..=20 个输出",
        ));
    }
    Ok(())
}

fn view_kind(value: &str) -> Option<VisualizationKind> {
    Some(match value {
        "line" => VisualizationKind::Line,
        "bar" => VisualizationKind::Bar,
        "scatter" => VisualizationKind::Scatter,
        "heatmap" => VisualizationKind::Heatmap,
        "boxplot" => VisualizationKind::Boxplot,
        "histogram" => VisualizationKind::Histogram,
        _ => return None,
    })
}

fn analysis_stages(active: Option<&str>, failed: bool) -> Vec<Stage> {
    let order = ["script", "run", "collect", "present"];
    [
        ("script", "生成脚本"),
        ("run", "执行分析"),
        ("collect", "收集结果"),
        ("present", "发布结果"),
    ]
    .into_iter()
    .map(|(key, label)| {
        let status = match active {
            None => StageStatus::Done,
            Some(value) if value == key && failed => StageStatus::Failed,
            Some(value) if value == key => StageStatus::Running,
            Some(value)
                if order.iter().position(|candidate| *candidate == key)
                    < order.iter().position(|candidate| *candidate == value) =>
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
