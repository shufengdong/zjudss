use crate::application::{
    AnalysisInput, AnalysisRequest, AnalysisService, EventBus, EventWriter, ResultColumn,
    ResultOutputRef, now_ms,
};
use crate::auth::Principal;
use crate::db::models::{NewTaskComparison, ResultDataset, TaskComparison};
use crate::db::repositories::{ComparisonRepo, DatasetRepo, MessageRepo, SimulationRepo};
use serde_json::{Value, json};
use std::collections::HashMap;
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
    tasks: SimulationRepo,
    datasets: DatasetRepo,
    analysis: AnalysisService,
    messages: MessageRepo,
    events: EventBus,
}
impl ComparisonService {
    pub fn new(
        repo: ComparisonRepo,
        tasks: SimulationRepo,
        datasets: DatasetRepo,
        analysis: AnalysisService,
        messages: MessageRepo,
        events: EventBus,
    ) -> Self {
        Self {
            repo,
            tasks,
            datasets,
            analysis,
            messages,
            events,
        }
    }
    pub async fn compare(
        &self,
        p: &Principal,
        session_id: i32,
        baseline_task_id: i32,
        candidate_task_id: i32,
        request_message_id: Option<i32>,
        skill_refs: Vec<Value>,
    ) -> Result<TaskComparison, ComparisonError> {
        if baseline_task_id == candidate_task_id {
            return Err(ComparisonError::new(
                "comparison_incompatible",
                "不能比较同一个任务",
            ));
        }
        let baseline = self
            .tasks
            .find(&p.access, baseline_task_id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        let candidate = self
            .tasks
            .find(&p.access, candidate_task_id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        if baseline.session_id != session_id || candidate.session_id != session_id {
            return Err(ComparisonError::new("not_found", "资源不存在"));
        }
        let left = self
            .datasets
            .list_task(&p.access, baseline_task_id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        let right = self
            .datasets
            .list_task(&p.access, candidate_task_id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        let (left, right, compatibility) = compatible_pair(&baseline.study_type, left, right)?;
        let analysis=self.analysis.run(AnalysisRequest{principal:p.clone(),session_id,request_message_id,intent:json!({"kind":"task_comparison","baselineTaskId":baseline_task_id.to_string(),"candidateTaskId":candidate_task_id.to_string(),"compatibility":compatibility}),script:comparison_script(&compatibility),inputs:vec![AnalysisInput{dataset_id:left.id,alias:"baseline".into()},AnalysisInput{dataset_id:right.id,alias:"candidate".into()}],expected_outputs:vec!["result".into()],initial_views:HashMap::new(),skill_refs}).await.map_err(|e|ComparisonError::new(e.code,e.to_string()))?;
        let comparison = self
            .repo
            .create(
                &p.access,
                NewTaskComparison {
                    session_id,
                    baseline_task_id,
                    candidate_task_id,
                    compatibility_json: crate::application::canonical_json(&compatibility),
                    analysis_run_id: analysis.run.id,
                    created_at_ms: now_ms(),
                },
            )
            .await
            .map_err(|e| ComparisonError::new("comparison_publish_failed", e.to_string()))?;
        let outputs = analysis.datasets.iter().map(result_ref).collect();
        EventWriter::new(
            p.access.clone(),
            session_id,
            self.messages.clone(),
            self.events.clone(),
        )
        .comparison(
            comparison.id,
            baseline_task_id,
            candidate_task_id,
            analysis.run.id,
            "仿真结果对比",
            outputs,
        )
        .await
        .map_err(|e| ComparisonError::new("comparison_event_failed", e.to_string()))?;
        Ok(comparison)
    }
    pub async fn find(
        &self,
        p: &Principal,
        id: i32,
    ) -> Result<(TaskComparison, Value, Vec<ResultDataset>), ComparisonError> {
        let c = self
            .repo
            .find(&p.access, id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        let (_, datasets) = self
            .analysis
            .find(p, c.analysis_run_id)
            .await
            .map_err(|_| ComparisonError::new("not_found", "资源不存在"))?;
        let compatibility = serde_json::from_str(&c.compatibility_json).unwrap_or(Value::Null);
        Ok((c, compatibility, datasets))
    }
}
fn compatible_pair(
    study: &str,
    left: Vec<ResultDataset>,
    right: Vec<ResultDataset>,
) -> Result<(ResultDataset, ResultDataset, Value), ComparisonError> {
    let by_name = right
        .into_iter()
        .map(|d| (d.result_name.clone(), d))
        .collect::<HashMap<_, _>>();
    for l in left {
        if let Some(r) = by_name.get(&l.result_name) {
            let lc: Vec<crate::application::datasets::DatasetColumn> =
                serde_json::from_str(&l.schema_json).unwrap_or_default();
            let rc: Vec<crate::application::datasets::DatasetColumn> =
                serde_json::from_str(&r.schema_json).unwrap_or_default();
            let rm = rc
                .iter()
                .map(|c| (c.name.as_str(), c))
                .collect::<HashMap<_, _>>();
            let mut keys = lc
                .iter()
                .filter(|c| {
                    let n = c.name.to_ascii_lowercase();
                    (n == "id" || n.ends_with("_id") || n == "name" || n == "time" || n == "time_s")
                        && rm.get(c.name.as_str()).is_some_and(|v| v.dtype == c.dtype)
                })
                .map(|c| c.name.clone())
                .collect::<Vec<_>>();
            if study.to_ascii_lowercase().contains("qsts")
                && !keys.iter().any(|k| k == "time" || k == "time_s")
            {
                continue;
            }
            if keys.is_empty() {
                continue;
            }
            keys.sort();
            let mut values = Vec::new();
            let mut unit_error = false;
            for c in &lc {
                if keys.contains(&c.name) || !numeric(&c.dtype) {
                    continue;
                }
                if let Some(other) = rm.get(c.name.as_str()) {
                    if other.dtype == c.dtype {
                        if other.unit != c.unit {
                            unit_error = true;
                            break;
                        }
                        values.push(json!({"column":c.name,"dtype":c.dtype,"unit":c.unit}));
                    }
                }
            }
            if unit_error {
                return Err(ComparisonError::new(
                    "comparison_incompatible",
                    "同名数值列单位不兼容",
                ));
            }
            if values.is_empty() {
                continue;
            }
            let compatibility = json!({"baselineDatasetId":l.id.to_string(),"candidateDatasetId":r.id.to_string(),"resultName":l.result_name,"stableKeys":keys,"values":values,"baselineSchema":lc,"candidateSchema":rc,"dimensions":{"baselineRows":l.row_count,"candidateRows":r.row_count}});
            return Ok((l, r.clone(), compatibility));
        }
    }
    Err(ComparisonError::new(
        "comparison_incompatible",
        "找不到具有稳定键、兼容数值列和单位的数据集",
    ))
}
fn numeric(v: &str) -> bool {
    ["Int", "UInt", "Float", "Decimal"]
        .iter()
        .any(|p| v.starts_with(p))
        || v == "int64"
        || v == "float64"
}
fn comparison_script(c: &Value) -> String {
    format!(
        "# controlled comparison\n# compatibility={}\nresult = compare_frames(baseline, candidate)\n",
        crate::application::canonical_json(c)
    )
}
fn result_ref(d: &ResultDataset) -> ResultOutputRef {
    let columns: Vec<crate::application::datasets::DatasetColumn> =
        serde_json::from_str(&d.schema_json).unwrap_or_default();
    ResultOutputRef {
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
        default_view_id: None,
        views: vec![],
    }
}
