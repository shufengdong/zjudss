use serde_json::{Value, json};

use crate::application::{
    AnalysisInput, AnalysisRequest, AnalysisService, EventBus, EventWriter, ResultOutputRef, now_ms,
};
use crate::auth::CurrentUser;
use crate::db::models::{NewRunComparison, RunComparison};
use crate::db::repositories::{ComparisonRepo, MessageRepo, ResultRepo, RunDataset, RunRepo};

mod compatibility;
mod script;

use compatibility::Compatibility;
use script::render_script;

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
        let pair = Compatibility::select(left, right)?;
        let compatibility =
            serde_json::to_value(&pair.facts).expect("compatibility facts are serializable");
        let analysis = self.analysis.run(AnalysisRequest {
            principal: principal.clone(),
            session_id,
            request_message_id,
            intent: json!({"kind":"run_comparison","baselineRunId":baseline_run_id.to_string(),"candidateRunId":candidate_run_id.to_string(),"compatibility":compatibility}),
            script: render_script(&pair.facts),
            inputs: vec![
                AnalysisInput { source: crate::application::AnalysisInputSource::DatasetId(pair.baseline.id), alias: "baseline".into() },
                AnalysisInput { source: crate::application::AnalysisInputSource::DatasetId(pair.candidate.id), alias: "candidate".into() },
            ],
            expected_outputs: vec!["result".into()],
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
        "scriptResourceId": run.script_resource_id.map(|id| id.to_string()),
        "scriptContentHash": run.script_content_hash,
        "engineContractVersion": run.engine_contract_version,
    })
    .to_string()
}

fn result_ref(dataset: &RunDataset) -> ResultOutputRef {
    ResultOutputRef::from_dataset(dataset)
}
