use std::{
    collections::{HashMap, HashSet},
    io::Cursor,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use serde_json::{Value, json};

use crate::adapters::{ENGINE_CONTRACT_VERSION, NamedInput, RunMode, RunSpec, SimEngine};
use crate::agent::knowledge::AgentKnowledge;
use crate::application::visualization::{VisualizationBudgets, VisualizationValidator};
use crate::application::{
    ArtifactMetadata, ArtifactService, DatasetViewRef, EventBus, EventWriter, OperationKind,
    OperationRef, ResultOutputRef, Stage, StageStatus, VisualizationKind, now_ms,
};
use crate::auth::CurrentUser;
use crate::db::models::{
    DatasetView, NewDatasetView, NewResultAnalysisRun, NewResultDataset, ResultAnalysisInput,
    ResultAnalysisRun,
};
use crate::db::repositories::{
    AnalysisRepo, DataResourceRepo, MessageRepo, ResultRepo, RunDataset,
};
use crate::storage::{ParquetInspector, StageLimits};

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
    pub initial_views: HashMap<String, Vec<(String, Value)>>,
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
        }
    }

    pub async fn run(&self, request: AnalysisRequest) -> Result<AnalysisOutcome, AnalysisError> {
        validate_request(&request)?;
        let input_rows = self.snapshot_inputs(&request).await?;
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
                input_rows,
            )
            .await
            .map_err(|error| AnalysisError::new("analysis_create_failed", error.to_string()))?;
        let root = self.data_dir.join("analysis-work").join(run.id.to_string());
        let result = self
            .run_inner(&request, &run, scope.id, &script.relative_path, &root)
            .await;
        let _ = std::fs::remove_dir_all(&root);
        match result {
            Ok((run, datasets)) => {
                for dataset in &datasets {
                    for (name, spec) in request
                        .initial_views
                        .get(&dataset.result_name)
                        .cloned()
                        .unwrap_or_default()
                    {
                        self.create_view(&request.principal, dataset.id, name, spec)
                            .await?;
                    }
                }
                Ok(AnalysisOutcome { run, datasets })
            }
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

    async fn snapshot_inputs(
        &self,
        request: &AnalysisRequest,
    ) -> Result<Vec<ResultAnalysisInput>, AnalysisError> {
        let mut rows = Vec::with_capacity(request.inputs.len());
        for (ordinal, input) in request.inputs.iter().enumerate() {
            match &input.source {
                AnalysisInputSource::DatasetId(dataset_id) => {
                    let dataset = self
                        .results
                        .find_live(&request.principal.id, *dataset_id)
                        .await
                        .map_err(|_| {
                            AnalysisError::new("analysis_input_not_found", "资源不存在")
                        })?;
                    if self
                        .results
                        .session_id(&request.principal.id, dataset.id)
                        .await
                        .map_err(|_| AnalysisError::new("analysis_input_not_found", "资源不存在"))?
                        != request.session_id
                    {
                        return Err(AnalysisError::new("analysis_input_not_found", "资源不存在"));
                    }
                    let artifact = self
                        .artifacts
                        .find_owned(
                            request.principal.storage_scope_id,
                            dataset.artifact_id.ok_or_else(|| {
                                AnalysisError::new("analysis_input_not_found", "资源不存在")
                            })?,
                        )
                        .await
                        .map_err(|_| {
                            AnalysisError::new("analysis_input_not_found", "资源不存在")
                        })?;
                    let producer = if let Some(run_id) = dataset.run_id {
                        json!({"kind":"run","runId":run_id.to_string()})
                    } else {
                        json!({"kind":"analysis","analysisRunId":dataset.analysis_run_id.map(|id|id.to_string())})
                    };
                    rows.push(ResultAnalysisInput {
                        analysis_run_id: 0,
                        dataset_id: Some(dataset.id),
                        resource_id: None,
                        alias: input.alias.clone(),
                        ordinal: ordinal as i32,
                        source_name: dataset.result_name,
                        source_schema_json: dataset.schema_json,
                        source_schema_hash: dataset.schema_hash,
                        source_content_hash: artifact.sha256,
                        source_producer_json: producer.to_string(),
                    });
                }
                AnalysisInputSource::DataResourceId(resource_id) => {
                    let resource = self
                        .data_resources
                        .find(&request.principal.id, resource_id, false)
                        .await
                        .map_err(|_| {
                            AnalysisError::new("analysis_input_not_found", "资源不存在")
                        })?;
                    if resource.resource.session_id != request.session_id {
                        return Err(AnalysisError::new("analysis_input_not_found", "资源不存在"));
                    }
                    rows.push(ResultAnalysisInput{analysis_run_id:0,dataset_id:None,resource_id:Some(resource.resource.id.clone()),alias:input.alias.clone(),ordinal:ordinal as i32,source_name:resource.resource.display_name,source_schema_json:resource.resource.schema_json,source_schema_hash:resource.resource.schema_hash,source_content_hash:resource.normalized_sha256.clone(),source_producer_json:json!({"kind":"data_resource","resourceId":resource.resource.id,"contentHash":resource.normalized_sha256}).to_string()});
                }
            }
        }
        Ok(rows)
    }

    async fn run_inner(
        &self,
        request: &AnalysisRequest,
        run: &ResultAnalysisRun,
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
        let mut named_inputs = Vec::new();
        for input in &request.inputs {
            let (artifact_id, input_key) = match &input.source {
                AnalysisInputSource::DatasetId(dataset_id) => {
                    let dataset = self
                        .results
                        .find_live(&request.principal.id, *dataset_id)
                        .await
                        .map_err(|error| {
                            AnalysisError::new("analysis_input_not_found", error.to_string())
                        })?;
                    (
                        dataset.artifact_id.ok_or_else(|| {
                            AnalysisError::new("analysis_input_not_found", "资源不存在")
                        })?,
                        format!("dataset-{dataset_id}"),
                    )
                }
                AnalysisInputSource::DataResourceId(resource_id) => {
                    let resource = self
                        .data_resources
                        .find(&request.principal.id, resource_id, false)
                        .await
                        .map_err(|error| {
                            AnalysisError::new("analysis_input_not_found", error.to_string())
                        })?;
                    (
                        resource.resource.normalized_artifact_id.ok_or_else(|| {
                            AnalysisError::new("analysis_input_not_found", "资源不存在")
                        })?,
                        format!("resource-{resource_id}"),
                    )
                }
            };
            let artifact = self
                .artifacts
                .find_owned(scope_id, artifact_id)
                .await
                .map_err(|error| {
                    AnalysisError::new("analysis_input_not_found", error.to_string())
                })?;
            let source = self
                .artifacts
                .store()
                .resolve(&artifact.storage_path)
                .map_err(|error| {
                    AnalysisError::new("analysis_input_not_found", error.to_string())
                })?;
            let relative_path = PathBuf::from(format!("inputs/{input_key}.parquet"));
            std::fs::copy(source, tree.join(&relative_path)).map_err(|error| {
                AnalysisError::new("analysis_staging_failed", error.to_string())
            })?;
            named_inputs.push(NamedInput {
                alias: input.alias.clone(),
                relative_path,
            });
        }
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
        let names = output
            .datasets
            .iter()
            .map(|dataset| dataset.name.as_str())
            .collect::<HashSet<_>>();
        if names.len() != request.expected_outputs.len()
            || request
                .expected_outputs
                .iter()
                .any(|name| !names.contains(name.as_str()))
        {
            return Err(AnalysisError::new(
                "analysis_output_mismatch",
                "实际输出与声明输出不一致",
            ));
        }
        self.stage(&request.principal, run.id, "collect", &writer)
            .await?;
        let mut committed = Vec::new();
        let mut drafts = Vec::new();
        for output in output.datasets {
            let metadata = ParquetInspector::metadata(&output.path).map_err(|error| {
                AnalysisError::new("analysis_output_invalid", error.to_string())
            })?;
            if !metadata.snappy {
                return Err(AnalysisError::new(
                    "analysis_output_invalid",
                    "输出必须为 Snappy Parquet",
                ));
            }
            let file = std::fs::File::open(&output.path).map_err(|error| {
                AnalysisError::new("analysis_output_invalid", error.to_string())
            })?;
            let staged = self
                .artifacts
                .store()
                .stage(
                    file,
                    StageLimits {
                        max_bytes: 500 * 1024 * 1024,
                    },
                )
                .map_err(|error| {
                    AnalysisError::new("analysis_output_invalid", error.to_string())
                })?;
            let artifact = match self
                .artifacts
                .commit(scope_id, staged, ArtifactMetadata::parquet())
                .await
            {
                Ok(artifact) => artifact,
                Err(error) => {
                    for artifact_id in committed {
                        let _ = self.artifacts.release(scope_id, artifact_id).await;
                    }
                    return Err(AnalysisError::new(
                        "analysis_output_commit_failed",
                        error.to_string(),
                    ));
                }
            };
            committed.push(artifact.id);
            let (schema_json, schema_hash) =
                crate::application::datasets::schema_json_and_hash(&metadata);
            drafts.push(NewResultDataset {
                run_id: None,
                analysis_run_id: Some(run.id),
                artifact_id: Some(artifact.id),
                result_name: output.name,
                schema_json,
                schema_hash,
                row_count: metadata.row_count as i64,
                size_bytes: metadata.size_bytes as i64,
                created_at_ms: now_ms(),
            });
        }
        self.stage(&request.principal, run.id, "present", &writer)
            .await?;
        let (run, datasets) = match self
            .repo
            .complete(&request.principal.id, run.id, drafts, now_ms())
            .await
        {
            Ok(value) => value,
            Err(error) => {
                for artifact_id in committed {
                    let _ = self.artifacts.release(scope_id, artifact_id).await;
                }
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

    pub async fn create_view(
        &self,
        principal: &CurrentUser,
        dataset_id: i32,
        name: String,
        spec: Value,
    ) -> Result<DatasetView, AnalysisError> {
        let source = self
            .results
            .find_live(&principal.id, dataset_id)
            .await
            .map_err(|error| AnalysisError::new("dataset_not_found", error.to_string()))?;
        let columns: Vec<crate::application::datasets::DatasetColumn> =
            serde_json::from_str(&source.schema_json)
                .map_err(|error| AnalysisError::new("dataset_schema_invalid", error.to_string()))?;
        let (parsed, _) = VisualizationValidator::parse_and_validate(
            &columns,
            source.row_count as u64,
            &spec,
            VisualizationBudgets::default(),
        )
        .map_err(|error| AnalysisError {
            code: error.code,
            message: error.message,
            details: error.details,
            run_id: None,
        })?;
        let session_id = self
            .results
            .session_id(&principal.id, dataset_id)
            .await
            .map_err(|error| AnalysisError::new("dataset_not_found", error.to_string()))?;
        let scope = self
            .artifacts
            .resolve_scope(principal.storage_scope_id)
            .await
            .map_err(|error| AnalysisError::new("storage_unavailable", error.to_string()))?;
        let source_artifact = self
            .artifacts
            .find_owned(
                scope.id,
                source
                    .artifact_id
                    .ok_or_else(|| AnalysisError::new("dataset_not_found", "资源不存在"))?,
            )
            .await
            .map_err(|error| AnalysisError::new("dataset_not_found", error.to_string()))?;
        let source_path = self
            .artifacts
            .store()
            .resolve(&source_artifact.storage_path)
            .map_err(|error| AnalysisError::new("dataset_not_found", error.to_string()))?;
        let staged = self
            .artifacts
            .store()
            .stage(
                std::fs::File::open(source_path).map_err(|error| {
                    AnalysisError::new("view_materialization_failed", error.to_string())
                })?,
                StageLimits {
                    max_bytes: 500 * 1024 * 1024,
                },
            )
            .map_err(|error| {
                AnalysisError::new("view_materialization_failed", error.to_string())
            })?;
        let materialized = self
            .artifacts
            .commit(scope.id, staged, ArtifactMetadata::parquet())
            .await
            .map_err(|error| {
                AnalysisError::new("view_materialization_failed", error.to_string())
            })?;
        let script_text = format!("# materialized view for dataset {dataset_id}\n");
        let script = self
            .commit_bytes(scope.id, script_text.as_bytes(), "rss", "text/plain")
            .await
            .map_err(|error| {
                AnalysisError::new("view_materialization_failed", error.to_string())
            })?;
        let producer = if let Some(run_id) = source.run_id {
            json!({"kind":"run","runId":run_id.to_string()})
        } else {
            json!({"kind":"analysis","analysisRunId":source.analysis_run_id.map(|id|id.to_string())})
        };
        let analysis = self.repo.create(&principal.id, NewResultAnalysisRun {
            session_id, request_message_id: None,
            intent_json: json!({"kind":"materialized_view","sourceDatasetId":dataset_id.to_string(),"spec":spec}).to_string(),
            script_artifact_id: script.id, status: "running".into(), stage: "present".into(), error_json: None,
            agent_bundle_version: self.knowledge.manifest.agent_bundle_version.clone(),
            agent_skill_refs_json: "[]".into(), engine_contract_version: ENGINE_CONTRACT_VERSION.into(),
            created_at_ms: now_ms(), started_at_ms: Some(now_ms()), finished_at_ms: None,
        }, vec![ResultAnalysisInput {
            analysis_run_id: 0, dataset_id:Some(dataset_id), resource_id:None, alias: "source".into(), ordinal: 0,
            source_name: source.result_name.clone(), source_schema_json: source.schema_json.clone(),
            source_schema_hash: source.schema_hash.clone(), source_content_hash:self.artifacts.find_owned(principal.storage_scope_id,source.artifact_id.unwrap()).await.map_err(|error|AnalysisError::new("view_materialization_failed",error.to_string()))?.sha256, source_producer_json: producer.to_string(),
        }]).await.map_err(|error| AnalysisError::new("view_materialization_failed", error.to_string()))?;
        let (_, datasets) = self
            .repo
            .complete(
                &principal.id,
                analysis.id,
                vec![NewResultDataset {
                    run_id: None,
                    analysis_run_id: Some(analysis.id),
                    artifact_id: Some(materialized.id),
                    result_name: format!("view_{dataset_id}_{}", analysis.id),
                    schema_json: source.schema_json.clone(),
                    schema_hash: source.schema_hash.clone(),
                    row_count: source.row_count,
                    size_bytes: source.size_bytes,
                    created_at_ms: now_ms(),
                }],
                now_ms(),
            )
            .await
            .map_err(|error| {
                AnalysisError::new("view_materialization_failed", error.to_string())
            })?;
        let view_dataset = datasets
            .into_iter()
            .next()
            .ok_or_else(|| AnalysisError::new("view_materialization_failed", "未生成视图数据集"))?;
        let view = self
            .repo
            .attach_view(
                &principal.id,
                dataset_id,
                NewDatasetView {
                    source_dataset_id: dataset_id,
                    view_dataset_id: view_dataset.id,
                    name,
                    kind: parsed.kind,
                    spec_json: crate::application::canonical_json(&spec),
                    is_default: false,
                    created_at_ms: now_ms(),
                },
            )
            .await
            .map_err(|error| AnalysisError::new("view_create_failed", error.to_string()))?;
        let writer = EventWriter::new(
            principal.id.clone(),
            session_id,
            self.messages.clone(),
            self.events.clone(),
        );
        writer
            .result(
                source.run_id,
                source.analysis_run_id,
                "数据集视图已更新",
                vec![self.result_ref(principal, &source).await?],
            )
            .await
            .map_err(|error| AnalysisError::new("analysis_event_failed", error.to_string()))?;
        Ok(view)
    }

    pub async fn create_resource_view(
        &self,
        principal: &CurrentUser,
        resource_id: &str,
        name: String,
        spec: Value,
    ) -> Result<DatasetView, AnalysisError> {
        let source = self
            .data_resources
            .find(&principal.id, resource_id, false)
            .await
            .map_err(|error| AnalysisError::new("data_resource_not_found", error.to_string()))?;
        let columns: Vec<crate::application::datasets::DatasetColumn> =
            serde_json::from_str(&source.resource.schema_json)
                .map_err(|error| AnalysisError::new("dataset_schema_invalid", error.to_string()))?;
        VisualizationValidator::parse_and_validate(
            &columns,
            source.resource.row_count as u64,
            &spec,
            VisualizationBudgets::default(),
        )
        .map_err(|error| AnalysisError {
            code: error.code,
            message: error.message,
            details: error.details,
            run_id: None,
        })?;

        let scope = self
            .artifacts
            .resolve_scope(principal.storage_scope_id)
            .await
            .map_err(|error| AnalysisError::new("storage_unavailable", error.to_string()))?;
        let source_artifact = self
            .artifacts
            .find_owned(
                scope.id,
                source
                    .resource
                    .normalized_artifact_id
                    .ok_or_else(|| AnalysisError::new("data_resource_not_found", "资源不存在"))?,
            )
            .await
            .map_err(|error| AnalysisError::new("data_resource_not_found", error.to_string()))?;
        let source_path = self
            .artifacts
            .store()
            .resolve(&source_artifact.storage_path)
            .map_err(|error| AnalysisError::new("data_resource_not_found", error.to_string()))?;
        let staged = self
            .artifacts
            .store()
            .stage(
                std::fs::File::open(source_path).map_err(|error| {
                    AnalysisError::new("view_materialization_failed", error.to_string())
                })?,
                StageLimits {
                    max_bytes: 500 * 1024 * 1024,
                },
            )
            .map_err(|error| {
                AnalysisError::new("view_materialization_failed", error.to_string())
            })?;
        let materialized = self
            .artifacts
            .commit(scope.id, staged, ArtifactMetadata::parquet())
            .await
            .map_err(|error| {
                AnalysisError::new("view_materialization_failed", error.to_string())
            })?;
        let script = self
            .commit_bytes(
                scope.id,
                format!("# materialized view source resource {resource_id}\n").as_bytes(),
                "rss",
                "text/plain",
            )
            .await
            .map_err(|error| {
                AnalysisError::new("view_materialization_failed", error.to_string())
            })?;
        let analysis = self
            .repo
            .create(
                &principal.id,
                NewResultAnalysisRun {
                    session_id: source.resource.session_id,
                    request_message_id: None,
                    intent_json: json!({
                        "kind": "materialized_view",
                        "sourceResourceId": resource_id,
                        "spec": spec,
                    })
                    .to_string(),
                    script_artifact_id: script.id,
                    status: "running".into(),
                    stage: "present".into(),
                    error_json: None,
                    agent_bundle_version: self.knowledge.manifest.agent_bundle_version.clone(),
                    agent_skill_refs_json: "[]".into(),
                    engine_contract_version: ENGINE_CONTRACT_VERSION.into(),
                    created_at_ms: now_ms(),
                    started_at_ms: Some(now_ms()),
                    finished_at_ms: None,
                },
                vec![ResultAnalysisInput {
                    analysis_run_id: 0,
                    dataset_id: None,
                    resource_id: Some(source.resource.id.clone()),
                    alias: "source".into(),
                    ordinal: 0,
                    source_name: source.resource.display_name.clone(),
                    source_schema_json: source.resource.schema_json.clone(),
                    source_schema_hash: source.resource.schema_hash.clone(),
                    source_content_hash: source.normalized_sha256.clone(),
                    source_producer_json: json!({
                        "kind": "data_resource",
                        "resourceId": resource_id,
                        "contentHash": source.normalized_sha256,
                    })
                    .to_string(),
                }],
            )
            .await
            .map_err(|error| {
                AnalysisError::new("view_materialization_failed", error.to_string())
            })?;
        let (_, datasets) = self
            .repo
            .complete(
                &principal.id,
                analysis.id,
                vec![NewResultDataset {
                    run_id: None,
                    analysis_run_id: Some(analysis.id),
                    artifact_id: Some(materialized.id),
                    result_name: format!("resource_{}_{}", &resource_id[..8], analysis.id),
                    schema_json: source.resource.schema_json,
                    schema_hash: source.resource.schema_hash,
                    row_count: source.resource.row_count,
                    size_bytes: source.normalized_size_bytes,
                    created_at_ms: now_ms(),
                }],
                now_ms(),
            )
            .await
            .map_err(|error| {
                AnalysisError::new("view_materialization_failed", error.to_string())
            })?;
        let dataset = datasets
            .into_iter()
            .next()
            .ok_or_else(|| AnalysisError::new("view_materialization_failed", "未生成视图数据集"))?;
        self.create_view(principal, dataset.id, name, spec).await
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
        output.default_view_id = views
            .iter()
            .find(|view| view.is_default)
            .map(|view| view.id.to_string());
        output.views = views
            .into_iter()
            .filter_map(|view| {
                view_kind(&view.kind).map(|kind| DatasetViewRef {
                    id: view.id.to_string(),
                    name: view.name,
                    kind,
                    is_default: view.is_default,
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
