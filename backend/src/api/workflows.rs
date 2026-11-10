//! 受管工作流查询与显式取消的 HTTP 边界。
//!
//! handler 只按当前用户读取稳定 workflow ID 或请求取消；候选生成、执行/修正预算、终态发布、
//! 重启恢复和日志脱敏由 managed workflow application service 拥有。

use axum::Json;
use axum::extract::{Path, State};

use crate::application::{AppState, ManagedWorkflowView};
use crate::auth::CurrentUser;

use super::ApiError;

/// `GET /api/workflows/{id}`：读取受管工作流的可恢复状态。
///
/// 成功返回 200、冻结配置 revision、attempt 摘要、可空脚本锚点、run 与结果；只返回脱敏有界
/// 诊断且不推进工作流。跨用户与不存在工作流统一返回 404。
pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<ManagedWorkflowView>, ApiError> {
    Ok(Json(state.managed_workflow_service.get(&user, id).await?))
}

/// `POST /api/workflows/{id}/cancel`：显式取消当前用户的受管工作流。
///
/// 非终态工作会被中断并收敛为 `cancelled_by_user`；终态或重复取消保持幂等，成功返回 200 和
/// 权威工作流。已有候选时终态仍原子发布最后候选脚本；跨用户与不存在工作流统一返回 404。
pub async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<ManagedWorkflowView>, ApiError> {
    Ok(Json(
        state.managed_workflow_service.cancel(&user, id).await?,
    ))
}
