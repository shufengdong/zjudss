//! 已持久化分析运行及其不可变输出的只读 HTTP 投影。
//!
//! 分析创建由受控 Agent/application 工具链拥有；本模块只按当前用户读取稳定 analysis ID，
//! 组合冻结运行记录与仍存活的结果数据集，不执行脚本、SQL 或任意查询。

use super::ApiError;
use crate::{
    application::{AppState, ms_to_iso},
    auth::CurrentUser,
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::Serialize;
use serde_json::Value;
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AnalysisDto {
    id: String,
    session_id: String,
    status: String,
    stage: String,
    intent: Value,
    script_artifact_id: String,
    agent_bundle_version: String,
    agent_skill_refs: Value,
    engine_contract_version: String,
    error: Option<Value>,
    created_at: String,
    started_at: Option<String>,
    finished_at: Option<String>,
    datasets: Vec<crate::application::datasets::DatasetMetadata>,
}
/// `GET /api/analysis-runs/{id}`：读取当前用户的分析记录与输出。
///
/// 成功返回 200、冻结 intent/provenance、状态、受控错误和存活数据集；读取不重跑分析，也不
/// 依赖 live 来源。跨用户、已删除与不存在分析统一返回 404。
pub async fn get(
    State(s): State<AppState>,
    p: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<AnalysisDto>, ApiError> {
    let (run, _) = s.analysis_service.find(&p, id).await?;
    let datasets = s.dataset_service.list_analysis(&p, id).await?;
    Ok(Json(AnalysisDto {
        id: run.id.to_string(),
        session_id: run.session_id.to_string(),
        status: run.status,
        stage: run.stage,
        intent: serde_json::from_str(&run.intent_json).unwrap_or(Value::Null),
        script_artifact_id: run.script_artifact_id.to_string(),
        agent_bundle_version: run.agent_bundle_version,
        agent_skill_refs: serde_json::from_str(&run.agent_skill_refs_json)
            .unwrap_or_else(|_| serde_json::json!([])),
        engine_contract_version: run.engine_contract_version,
        error: run
            .error_json
            .as_deref()
            .and_then(|v| serde_json::from_str(v).ok()),
        created_at: ms_to_iso(run.created_at_ms),
        started_at: run.started_at_ms.map(ms_to_iso),
        finished_at: run.finished_at_ms.map(ms_to_iso),
        datasets,
    }))
}
