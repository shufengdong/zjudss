//! 已持久化运行比较与物化输出的只读 HTTP 投影。
//!
//! 比较创建、兼容性判断和受控 RSS 生成由 comparison application service 拥有；本模块只按
//! 当前用户组合冻结比较记录、兼容性快照和对应分析结果，不接收 SQL 或服务器路径。

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
pub struct ComparisonDto {
    id: String,
    session_id: String,
    baseline_run_id: String,
    candidate_run_id: String,
    compatibility: Value,
    analysis_run_id: String,
    created_at: String,
    datasets: Vec<crate::application::datasets::DatasetMetadata>,
}
/// `GET /api/run-comparisons/{id}`：读取当前用户的运行比较及派生结果。
///
/// 成功返回 200、冻结 baseline/candidate ID、兼容性快照、analysis ID 与存活输出；读取不重算
/// 比较，来源 tombstone 后仍以已物化快照为准。跨用户与不存在比较统一返回 404。
pub async fn get(
    State(s): State<AppState>,
    p: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<ComparisonDto>, ApiError> {
    let (c, compatibility, _) = s.comparison_service.find(&p, id).await?;
    let datasets = s
        .dataset_service
        .list_analysis(&p, c.analysis_run_id)
        .await?;
    Ok(Json(ComparisonDto {
        id: c.id.to_string(),
        session_id: c.session_id.to_string(),
        baseline_run_id: c.baseline_run_id.to_string(),
        candidate_run_id: c.candidate_run_id.to_string(),
        compatibility,
        analysis_run_id: c.analysis_run_id.to_string(),
        created_at: ms_to_iso(c.created_at_ms),
        datasets,
    }))
}
