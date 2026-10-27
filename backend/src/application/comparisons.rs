use std::collections::HashMap;

use serde_json::{Value, json};

use crate::application::{
    AnalysisInput, AnalysisRequest, AnalysisService, EventBus, EventWriter, ResultOutputRef, now_ms,
};
use crate::auth::CurrentUser;
use crate::db::models::{NewRunComparison, RunComparison};
use crate::db::repositories::{ComparisonRepo, MessageRepo, ResultRepo, RunDataset, RunRepo};

#[derive(Debug)]
pub struct ComparisonError {
    pub code: &'static str,
    pub message: String,
}

impl ComparisonError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for ComparisonError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ComparisonError {}

#[derive(Clone)]
pub struct ComparisonService {
    repo: ComparisonRepo,
    runs: RunRepo,
    datasets: ResultRepo,
    analysis: AnalysisService,
    messages: MessageRepo,
    events: EventBus,
}

impl ComparisonService {
    pub fn new(
        repo: ComparisonRepo,
        runs: RunRepo,
        datasets: ResultRepo,
        analysis: AnalysisService,
        messages: MessageRepo,
        events: EventBus,
    ) -> Self {
        Self {
            repo,
            runs,
            datasets,
            analysis,
            messages,
            events,
        }
    }

    pub async fn compare(
        &self,
        principal: &CurrentUser,
        session_id: i32,
        baseline_run_id: i32,
        candidate_run_id: i32,
        request_message_id: Option<i32>,
        skill_refs: Vec<Value>,
    ) -> Result<RunComparison, ComparisonError> {
        if baseline_run_id == candidate_run_id {
            return Err(ComparisonError::new(
                "comparison_incompatible",
                "不能比较同一次运行",
            ));
        }
        let baseline = self
            .runs
            .find(&principal.id, baseline_run_id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        let candidate = self
            .runs
            .find(&principal.id, candidate_run_id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        if baseline.session_id != session_id
            || candidate.session_id != session_id
            || baseline.status != "succeeded"
            || candidate.status != "succeeded"
        {
            return Err(ComparisonError::new("not_found", "资源不存在"));
        }
        let left = self
            .datasets
            .list_run(&principal.id, baseline_run_id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        let right = self
            .datasets
            .list_run(&principal.id, candidate_run_id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        let (left, right, compatibility) = compatible_pair(left, right)?;
        let analysis = self.analysis.run(AnalysisRequest {
            principal: principal.clone(),
            session_id,
            request_message_id,
            intent: json!({"kind":"run_comparison","baselineRunId":baseline_run_id.to_string(),"candidateRunId":candidate_run_id.to_string(),"compatibility":compatibility}),
            script: comparison_script(&compatibility),
            inputs: vec![
                AnalysisInput { dataset_id: left.id, alias: "baseline".into() },
                AnalysisInput { dataset_id: right.id, alias: "candidate".into() },
            ],
            expected_outputs: vec!["result".into()],
            initial_views: HashMap::new(),
            skill_refs,
        }).await.map_err(|error| ComparisonError::new(error.code, error.to_string()))?;
        let comparison = self
            .repo
            .create(
                &principal.id,
                NewRunComparison {
                    session_id,
                    baseline_run_id,
                    candidate_run_id,
                    baseline_snapshot_json: run_snapshot(&baseline),
                    candidate_snapshot_json: run_snapshot(&candidate),
                    compatibility_json: crate::application::canonical_json(&compatibility),
                    analysis_run_id: analysis.run.id,
                    created_at_ms: now_ms(),
                },
            )
            .await
            .map_err(|error| {
                ComparisonError::new("comparison_publish_failed", error.to_string())
            })?;
        EventWriter::new(
            principal.id.clone(),
            session_id,
            self.messages.clone(),
            self.events.clone(),
        )
        .comparison(
            comparison.id,
            baseline_run_id,
            candidate_run_id,
            analysis.run.id,
            "运行结果对比",
            analysis.datasets.iter().map(result_ref).collect(),
        )
        .await
        .map_err(|error| ComparisonError::new("comparison_event_failed", error.to_string()))?;
        Ok(comparison)
    }

    pub async fn find(
        &self,
        principal: &CurrentUser,
        id: i32,
    ) -> Result<(RunComparison, Value, Vec<RunDataset>), ComparisonError> {
        let comparison = self
            .repo
            .find(&principal.id, id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        let (_, datasets) = self
            .analysis
            .find(principal, comparison.analysis_run_id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        let compatibility =
            serde_json::from_str(&comparison.compatibility_json).unwrap_or(Value::Null);
        Ok((comparison, compatibility, datasets))
    }
}

fn run_snapshot(run: &crate::db::models::SimulationRun) -> String {
    json!({
        "runId": run.id.to_string(),
        "modelVersionId": run.model_version_id.map(|id| id.to_string()),
        "modelContentHash": run.model_content_hash,
        "scriptResourceId": run.script_resource_id.to_string(),
        "scriptContentHash": run.script_content_hash,
        "engineContractVersion": run.engine_contract_version,
    })
    .to_string()
}

fn compatible_pair(
    left: Vec<RunDataset>,
    right: Vec<RunDataset>,
) -> Result<(RunDataset, RunDataset, Value), ComparisonError> {
    let by_name = right
        .into_iter()
        .map(|dataset| (dataset.result_name.clone(), dataset))
        .collect::<HashMap<_, _>>();
    for baseline in left {
        let Some(candidate) = by_name.get(&baseline.result_name) else {
            continue;
        };
        let baseline_columns: Vec<crate::application::datasets::DatasetColumn> =
            serde_json::from_str(&baseline.schema_json).unwrap_or_default();
        let candidate_columns: Vec<crate::application::datasets::DatasetColumn> =
            serde_json::from_str(&candidate.schema_json).unwrap_or_default();
        let candidate_by_name = candidate_columns
            .iter()
            .map(|column| (column.name.as_str(), column))
            .collect::<HashMap<_, _>>();
        let mut keys = baseline_columns
            .iter()
            .filter(|column| {
                let name = column.name.to_ascii_lowercase();
                (name == "id"
                    || name == "key"
                    || name.ends_with("_id")
                    || name == "name"
                    || name == "bus"
                    || name == "kind"
                    || name == "time"
                    || name == "time_s")
                    && rss_identifier(&column.name)
                    && candidate_by_name
                        .get(column.name.as_str())
                        .is_some_and(|other| other.dtype == column.dtype)
            })
            .map(|column| column.name.clone())
            .collect::<Vec<_>>();
        if keys.is_empty() {
            continue;
        }
        keys.sort();
        let mut values = Vec::new();
        for column in &baseline_columns {
            if keys.contains(&column.name)
                || !numeric(&column.dtype)
                || !rss_identifier(&column.name)
            {
                continue;
            }
            let Some(other) = candidate_by_name.get(column.name.as_str()) else {
                continue;
            };
            if other.dtype != column.dtype {
                continue;
            }
            if other.unit != column.unit {
                return Err(ComparisonError::new(
                    "comparison_incompatible",
                    "同名数值列单位不兼容",
                ));
            }
            values.push(json!({"column":column.name,"dtype":column.dtype,"unit":column.unit}));
        }
        if values.is_empty() {
            continue;
        }
        let compatibility = json!({
            "baselineDatasetId": baseline.id.to_string(),
            "candidateDatasetId": candidate.id.to_string(),
            "resultName": baseline.result_name,
            "stableKeys": keys,
            "values": values,
            "baselineSchema": baseline_columns,
            "candidateSchema": candidate_columns,
            "dimensions": {"baselineRows":baseline.row_count,"candidateRows":candidate.row_count},
        });
        return Ok((baseline, candidate.clone(), compatibility));
    }
    Err(ComparisonError::new(
        "comparison_incompatible",
        "找不到具有稳定键、兼容数值列和单位的数据集",
    ))
}

fn numeric(value: &str) -> bool {
    ["Int", "UInt", "Float", "Decimal"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
        || value == "int64"
        || value == "float64"
}

fn rss_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn comparison_script(compatibility: &Value) -> String {
    let keys = serde_json::to_string(compatibility.get("stableKeys").unwrap_or(&json!([])))
        .unwrap_or_else(|_| "[]".into());
    let values = compatibility
        .get("values")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|value| value.get("column").and_then(Value::as_str))
        .collect::<Vec<_>>();
    let values = serde_json::to_string(&values).unwrap_or_else(|_| "[]".into());
    format!("result = compare_frames(baseline, candidate, keys={keys}, values={values})\n")
}

fn result_ref(dataset: &RunDataset) -> ResultOutputRef {
    ResultOutputRef::from_dataset(dataset)
}
