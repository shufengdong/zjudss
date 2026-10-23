use crate::adapters::{ENGINE_CONTRACT_VERSION, NamedInput, RunMode, RunSpec, SimEngine};
use crate::agent::knowledge::AgentKnowledge;
use crate::application::visualization::{VisualizationBudgets, VisualizationValidator};
use crate::application::{
    ArtifactMetadata, ArtifactService, DatasetViewRef, EventBus, EventWriter, OperationKind,
    OperationRef, ResultColumn, ResultOutputRef, Stage, StageStatus, VisualizationKind, now_ms,
};
use crate::auth::CurrentUser;
use crate::db::models::{
    NewDatasetView, NewResultAnalysisRun, NewResultDataset, ResultAnalysisInput, ResultAnalysisRun,
    ResultDataset,
};
use crate::db::repositories::{AnalysisRepo, DatasetRepo, MessageRepo};
use crate::storage::{ParquetInspector, StageLimits};
use serde_json::{Value, json};
use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

#[derive(Debug, Clone)]
pub struct AnalysisInput {
    pub dataset_id: i32,
    pub alias: String,
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
    pub initial_views: HashMap<String, Vec<(String, Value)>>,
    pub skill_refs: Vec<Value>,
}
#[derive(Debug, Clone)]
pub struct AnalysisOutcome {
    pub run: ResultAnalysisRun,
    pub datasets: Vec<ResultDataset>,
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
pub struct AnalysisService {
    repo: AnalysisRepo,
    datasets: DatasetRepo,
    artifacts: ArtifactService,
    messages: MessageRepo,
    events: EventBus,
    engine: Arc<dyn SimEngine>,
    knowledge: Arc<AgentKnowledge>,
    data_dir: PathBuf,
}
impl AnalysisService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        repo: AnalysisRepo,
        datasets: DatasetRepo,
        artifacts: ArtifactService,
        messages: MessageRepo,
        events: EventBus,
        engine: Arc<dyn SimEngine>,
        knowledge: Arc<AgentKnowledge>,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            repo,
            datasets,
            artifacts,
            messages,
            events,
            engine,
            knowledge,
            data_dir,
        }
    }
    pub async fn run(&self, r: AnalysisRequest) -> Result<AnalysisOutcome, AnalysisError> {
        validate_request(&r)?;
        let scope = self
            .artifacts
            .resolve_scope(r.principal.storage_scope_id)
            .await
            .map_err(|e| AnalysisError::new("storage_unavailable", e.to_string()))?;
        let staged = self
            .artifacts
            .store()
            .stage(
                Cursor::new(r.script.as_bytes()),
                StageLimits {
                    max_bytes: 2 * 1024 * 1024,
                },
            )
            .map_err(|e| AnalysisError::new("analysis_script_invalid", e.to_string()))?;
        let script = self
            .artifacts
            .commit(
                scope.id,
                staged,
                ArtifactMetadata {
                    format: "rss".into(),
                    media_type: Some("text/plain".into()),
                },
            )
            .await
            .map_err(|e| AnalysisError::new("analysis_script_commit_failed", e.to_string()))?;
        let inputs = r
            .inputs
            .iter()
            .enumerate()
            .map(|(i, v)| ResultAnalysisInput {
                analysis_run_id: 0,
                dataset_id: v.dataset_id,
                alias: v.alias.clone(),
                ordinal: i as i32,
            })
            .collect();
        let run = self
            .repo
            .create(
                &r.principal.id,
                NewResultAnalysisRun {
                    session_id: r.session_id,
                    request_message_id: r.request_message_id,
                    intent_json: crate::application::canonical_json(&r.intent),
                    script_artifact_id: script.id,
                    status: "pending".into(),
                    stage: "script".into(),
                    error_json: None,
                    agent_bundle_version: self.knowledge.manifest.agent_bundle_version.clone(),
                    agent_skill_refs_json: crate::application::canonical_json(&Value::Array(
                        r.skill_refs.clone(),
                    )),
                    engine_contract_version: ENGINE_CONTRACT_VERSION.into(),
                    created_at_ms: now_ms(),
                    started_at_ms: None,
                    finished_at_ms: None,
                },
                inputs,
            )
            .await
            .map_err(|e| AnalysisError::new("analysis_create_failed", e.to_string()))?;
        let root = self.data_dir.join("analysis-work").join(run.id.to_string());
        let result = self
            .run_inner(&r, &run, scope.id, &script.relative_path, &root)
            .await;
        let _ = std::fs::remove_dir_all(&root);
        match result {
            Ok((run, datasets)) => Ok(AnalysisOutcome { run, datasets }),
            Err(e) => {
                let _ = self
                    .repo
                    .fail(
                        &r.principal.id,
                        run.id,
                        "present",
                        json!({"code":e.code,"message":e.message}).to_string(),
                        now_ms(),
                    )
                    .await;
                Err(e.run(run.id))
            }
        }
    }
    async fn run_inner(
        &self,
        r: &AnalysisRequest,
        run: &ResultAnalysisRun,
        scope_id: i32,
        script_artifact_path: &str,
        root: &Path,
    ) -> Result<(ResultAnalysisRun, Vec<ResultDataset>), AnalysisError> {
        let tree = root.join("input");
        let results = root.join("results-staging");
        std::fs::create_dir_all(&tree)
            .and_then(|_| std::fs::create_dir_all(&results))
            .map_err(|e| AnalysisError::new("analysis_staging_failed", e.to_string()))?;
        let writer = EventWriter::new(
            r.principal.id.clone(),
            r.session_id,
            self.messages.clone(),
            self.events.clone(),
        );
        self.stage(&r.principal, run.id, "script", &writer).await?;
        let source = self
            .artifacts
            .store()
            .resolve(script_artifact_path)
            .map_err(|e| AnalysisError::new("analysis_script_missing", e.to_string()))?;
        std::fs::create_dir_all(tree.join("generated"))
            .map_err(|e| AnalysisError::new("analysis_staging_failed", e.to_string()))?;
        std::fs::copy(source, tree.join("generated/analysis.rss"))
            .map_err(|e| AnalysisError::new("analysis_staging_failed", e.to_string()))?;
        let mut named = Vec::new();
        for input in &r.inputs {
            let d = self
                .datasets
                .find(&r.principal.id, input.dataset_id)
                .await
                .map_err(|e| AnalysisError::new("analysis_input_not_found", e.to_string()))?;
            let a = self
                .artifacts
                .find_owned(scope_id, d.artifact_id)
                .await
                .map_err(|e| AnalysisError::new("analysis_input_not_found", e.to_string()))?;
            let source = self
                .artifacts
                .store()
                .resolve(&a.storage_path)
                .map_err(|e| AnalysisError::new("analysis_input_not_found", e.to_string()))?;
            let rel = PathBuf::from(format!("inputs/{}.parquet", d.id));
            std::fs::create_dir_all(tree.join("inputs"))
                .map_err(|e| AnalysisError::new("analysis_staging_failed", e.to_string()))?;
            std::fs::copy(source, tree.join(&rel))
                .map_err(|e| AnalysisError::new("analysis_staging_failed", e.to_string()))?;
            named.push(NamedInput {
                alias: input.alias.clone(),
                relative_path: rel,
            });
        }
        self.stage(&r.principal, run.id, "run", &writer).await?;
        let output = self
            .engine
            .run_analysis(RunSpec {
                mode: RunMode::Analysis,
                input_tree: tree,
                script_path: PathBuf::from("generated/analysis.rss"),
                named_inputs: named,
                result_dir: results,
                timeout: Duration::from_secs(300),
            })
            .await
            .map_err(|e| AnalysisError::new("analysis_engine_failed", e.to_string()))?;
        let names = output
            .datasets
            .iter()
            .map(|v| v.name.as_str())
            .collect::<HashSet<_>>();
        if names.len() != r.expected_outputs.len()
            || r.expected_outputs
                .iter()
                .any(|v| !names.contains(v.as_str()))
        {
            return Err(AnalysisError::new(
                "analysis_output_mismatch",
                "实际输出与声明输出不一致",
            ));
        }
        self.stage(&r.principal, run.id, "collect", &writer).await?;
        let mut committed = Vec::new();
        let mut drafts = Vec::new();
        for out in output.datasets {
            let meta = ParquetInspector::metadata(&out.path)
                .map_err(|e| AnalysisError::new("analysis_output_invalid", e.to_string()))?;
            if !meta.snappy {
                return Err(AnalysisError::new(
                    "analysis_output_invalid",
                    "输出必须为 Snappy Parquet",
                ));
            }
            let staged = self
                .artifacts
                .store()
                .stage(
                    std::fs::File::open(&out.path).map_err(|e| {
                        AnalysisError::new("analysis_output_invalid", e.to_string())
                    })?,
                    StageLimits {
                        max_bytes: 500 * 1024 * 1024,
                    },
                )
                .map_err(|e| AnalysisError::new("analysis_output_invalid", e.to_string()))?;
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
                    return Err(AnalysisError::new(
                        "analysis_output_commit_failed",
                        e.to_string(),
                    ));
                }
            };
            committed.push(artifact.id);
            let (schema_json, schema_hash) =
                crate::application::datasets::schema_json_and_hash(&meta);
            let columns: Vec<crate::application::datasets::DatasetColumn> =
                serde_json::from_str(&schema_json).unwrap();
            let mut views = Vec::new();
            for (name, spec) in r.initial_views.get(&out.name).cloned().unwrap_or_default() {
                let (parsed, _) = VisualizationValidator::parse_and_validate(
                    &columns,
                    meta.row_count,
                    &spec,
                    VisualizationBudgets::default(),
                )
                .map_err(|e| AnalysisError {
                    code: e.code,
                    message: e.message,
                    details: e.details,
                    run_id: None,
                })?;
                views.push(NewDatasetView {
                    dataset_id: 0,
                    name,
                    kind: parsed.kind,
                    spec_json: crate::application::canonical_json(&spec),
                    is_default: views.is_empty(),
                    created_at_ms: now_ms(),
                });
            }
            drafts.push((
                NewResultDataset {
                    task_id: None,
                    analysis_run_id: Some(run.id),
                    artifact_id: artifact.id,
                    result_name: out.name,
                    schema_json,
                    schema_hash,
                    row_count: meta.row_count as i64,
                    size_bytes: meta.size_bytes as i64,
                    created_at_ms: now_ms(),
                },
                views,
            ));
        }
        self.stage(&r.principal, run.id, "present", &writer).await?;
        let (run, datasets) = match self
            .repo
            .complete(&r.principal.id, run.id, drafts, now_ms())
            .await
        {
            Ok(v) => v,
            Err(e) => {
                for id in committed {
                    let _ = self.artifacts.release(scope_id, id).await;
                }
                return Err(AnalysisError::new("analysis_publish_failed", e.to_string()));
            }
        };
        let mut outputs = Vec::new();
        for dataset in &datasets {
            outputs.push(self.result_ref(&r.principal, dataset).await?);
        }
        writer
            .result(None, Some(run.id), "结果分析", outputs)
            .await
            .map_err(|e| AnalysisError::new("analysis_event_failed", e.to_string()))?;
        writer
            .progress(
                OperationRef {
                    kind: OperationKind::Analysis,
                    id: run.id.to_string(),
                },
                analysis_stages(None, false),
            )
            .await
            .map_err(|e| AnalysisError::new("analysis_event_failed", e.to_string()))?;
        Ok((run, datasets))
    }
    async fn stage(
        &self,
        p: &CurrentUser,
        id: i32,
        name: &str,
        w: &EventWriter,
    ) -> Result<(), AnalysisError> {
        self.repo
            .set_stage(&p.id, id, name, now_ms())
            .await
            .map_err(|e| AnalysisError::new("analysis_update_failed", e.to_string()))?;
        w.progress(
            OperationRef {
                kind: OperationKind::Analysis,
                id: id.to_string(),
            },
            analysis_stages(Some(name), false),
        )
        .await
        .map_err(|e| AnalysisError::new("analysis_event_failed", e.to_string()))?;
        Ok(())
    }
    pub async fn create_view(
        &self,
        p: &CurrentUser,
        dataset_id: i32,
        name: String,
        spec: Value,
    ) -> Result<crate::db::models::DatasetView, AnalysisError> {
        let d = self
            .datasets
            .find(&p.id, dataset_id)
            .await
            .map_err(|e| AnalysisError::new("dataset_not_found", e.to_string()))?;
        let columns: Vec<crate::application::datasets::DatasetColumn> =
            serde_json::from_str(&d.schema_json)
                .map_err(|e| AnalysisError::new("dataset_schema_invalid", e.to_string()))?;
        let (parsed, _) = VisualizationValidator::parse_and_validate(
            &columns,
            d.row_count as u64,
            &spec,
            VisualizationBudgets::default(),
        )
        .map_err(|e| AnalysisError {
            code: e.code,
            message: e.message,
            details: e.details,
            run_id: None,
        })?;
        let view = self
            .repo
            .create_view(
                &p.id,
                dataset_id,
                NewDatasetView {
                    dataset_id,
                    name,
                    kind: parsed.kind,
                    spec_json: crate::application::canonical_json(&spec),
                    is_default: false,
                    created_at_ms: now_ms(),
                },
            )
            .await
            .map_err(|e| AnalysisError::new("view_create_failed", e.to_string()))?;
        let session_id = self
            .datasets
            .session_id(&p.id, dataset_id)
            .await
            .map_err(|e| AnalysisError::new("dataset_not_found", e.to_string()))?;
        let writer = EventWriter::new(
            p.id.clone(),
            session_id,
            self.messages.clone(),
            self.events.clone(),
        );
        writer
            .result(
                d.task_id,
                d.analysis_run_id,
                "数据集视图已更新",
                vec![self.result_ref(p, &d).await?],
            )
            .await
            .map_err(|e| AnalysisError::new("analysis_event_failed", e.to_string()))?;
        Ok(view)
    }
    pub async fn find(
        &self,
        p: &CurrentUser,
        id: i32,
    ) -> Result<(ResultAnalysisRun, Vec<ResultDataset>), AnalysisError> {
        let run = self
            .repo
            .find(&p.id, id)
            .await
            .map_err(|e| AnalysisError::new("analysis_not_found", e.to_string()))?;
        let ds = self
            .datasets
            .list_analysis(&p.id, id)
            .await
            .map_err(|e| AnalysisError::new("analysis_not_found", e.to_string()))?;
        Ok((run, ds))
    }
    async fn result_ref(
        &self,
        p: &CurrentUser,
        d: &ResultDataset,
    ) -> Result<ResultOutputRef, AnalysisError> {
        let columns: Vec<crate::application::datasets::DatasetColumn> =
            serde_json::from_str(&d.schema_json).unwrap_or_default();
        let views = self
            .datasets
            .views(&p.id, d.id)
            .await
            .map_err(|e| AnalysisError::new("dataset_view_read_failed", e.to_string()))?;
        let default_view_id = views
            .iter()
            .find(|v| v.is_default)
            .map(|v| v.id.to_string());
        Ok(ResultOutputRef {
            dataset_id: d.id.to_string(),
            name: d.result_name.clone(),
            row_count: d.row_count,
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
            preview_truncated: d.row_count > 0,
            default_view_id,
            views: views
                .into_iter()
                .filter_map(|v| {
                    view_kind(&v.kind).map(|kind| DatasetViewRef {
                        id: v.id.to_string(),
                        name: v.name,
                        kind,
                        is_default: v.is_default,
                    })
                })
                .collect(),
        })
    }
}
fn validate_request(r: &AnalysisRequest) -> Result<(), AnalysisError> {
    if r.script.len() > 2 * 1024 * 1024
        || r.script.contains("http://")
        || r.script.contains("https://")
        || r.script.contains("../")
        || r.script.contains("import \"")
        || r.script.contains("include \"")
    {
        return Err(AnalysisError::new(
            "analysis_script_unsafe",
            "脚本包含远程或非绑定路径",
        ));
    }
    if r.inputs.is_empty() || r.inputs.len() > 20 {
        return Err(AnalysisError::new(
            "analysis_inputs_invalid",
            "输入数量必须为 1..=20",
        ));
    }
    let mut aliases = HashSet::new();
    for i in &r.inputs {
        let mut chars = i.alias.chars();
        if !chars
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
            || !chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
            || !aliases.insert(i.alias.clone())
        {
            return Err(AnalysisError::new("analysis_alias_invalid", &i.alias));
        }
    }
    if r.expected_outputs.is_empty() || r.expected_outputs.len() > 20 {
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
    .map(|(k, l)| {
        let status = match active {
            None => StageStatus::Done,
            Some(v) if v == k && failed => StageStatus::Failed,
            Some(v) if v == k => StageStatus::Running,
            Some(v) if order.iter().position(|x| *x == k) < order.iter().position(|x| *x == v) => {
                StageStatus::Done
            }
            Some(_) => StageStatus::Pending,
        };
        Stage {
            key: k.into(),
            label: l.into(),
            status,
        }
    })
    .collect()
}
