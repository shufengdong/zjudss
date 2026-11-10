//! 当前用户 artifact 存储用量查询。
//!
//! 该受保护路由只把认证用户的 storage scope 交给 artifact service；配额核算和 artifact 引用
//! 规则仍由 application/repository 层拥有。

use axum::Json;
use axum::extract::State;

use crate::application::{AppState, StorageUsageSummary};
use crate::auth::CurrentUser;

use super::ApiError;

/// `GET /api/storage-usage`：读取当前用户存储 scope 的用量摘要。
///
/// 成功返回 200 和 [`StorageUsageSummary`]；该接口只读，不扫描或泄露其他用户的 artifact。
pub async fn usage(
    State(state): State<AppState>,
    principal: CurrentUser,
) -> Result<Json<StorageUsageSummary>, ApiError> {
    state
        .artifact_service
        .usage_for(principal.storage_scope_id)
        .await
        .map(Json)
        .map_err(Into::into)
}
