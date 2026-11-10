//! 受管工作流输入差异重试的固定确认入口。
//!
//! 本模块只消费短期 opaque preview ID 和明确 `confirmed` 决定；最新输入冻结、私有 Struct
//! overlay、artifact 提交与派生 workflow 事务由 managed workflow service 拥有。

use axum::Json;
use axum::extract::{Path, State};

use crate::application::{AppState, ConfirmRetryRequest, ManagedWorkflowView};
use crate::auth::CurrentUser;

use super::ApiError;

/// `POST /api/retry-previews/{id}/confirm`：确认输入变化并创建派生工作流。
///
/// 请求必须明确 `confirmed=true`，preview ID 必须属于当前用户、未过期且仍匹配 engine/facade
/// 合同。成功冻结最新输入、原子创建 workflow 并返回 200；无效/过期 preview 或合同变化
/// 返回稳定 422，跨用户与不存在 preview 统一隐藏为 404。
pub async fn confirm(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<String>,
    Json(body): Json<ConfirmRetryRequest>,
) -> Result<Json<ManagedWorkflowView>, ApiError> {
    Ok(Json(
        state
            .managed_workflow_service
            .confirm_retry(&user, &id, body)
            .await?,
    ))
}
