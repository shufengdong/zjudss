//! revisioned 研究配置卡与固定按钮确认的 HTTP 边界。
//!
//! 所有 handler 按当前用户解析配置，并把精确 `expectedRevision` 交给 study/workflow service。
//! 输入绑定、Struct 私有覆盖、结果约束、确认事务与执行调度由 application 层拥有；本模块在
//! 每次成功读取或变更后同步 durable `study-config` 卡片。最终确认只对应固定按钮路由，不是
//! 对话 Agent 的隐式动作。

use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Deserialize;

use crate::application::{
    AppState, BindStudyResourceRequest, ConfirmStudyRequest, EventWriter, ManagedWorkflowView,
    SelectFacadeRequest, SelectStudyResultsRequest, StudyConfigCard, StudyConfigurationView,
    UpdateStudyStructRequest,
};
use crate::auth::CurrentUser;

use super::ApiError;

/// `GET /api/study-configurations/{id}`：读取当前用户的研究配置卡。
///
/// 成功返回 200 和权威 revision/status，并幂等同步 durable 卡片；不会确认配置或启动执行。
/// 跨用户与不存在配置统一返回 404。
pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state.study_configuration_service.get(&user, id).await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

/// `PUT /api/study-configurations/{id}/facade`：按 revision 更换目录门面。
///
/// 请求携带精确 `expectedRevision` 和稳定 facade ID；成功重建受控输入/结果卡、递增 revision、
/// 同步 durable 卡片并返回 200。过期 revision 返回 409 `configuration_stale`，禁用或不兼容
/// 门面返回受控 422，跨用户与不存在配置统一返回 404。
pub async fn select_facade(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<SelectFacadeRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .select_facade(&user, id, body)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

/// `PUT /api/study-configurations/{id}/bindings/{inputId}`：选择或清除普通资源绑定。
///
/// 请求必须携带精确 revision，并只接受当前用户的稳定 resource ID；成功递增 revision、同步
/// 卡片并返回 200。资源不兼容或选择无效返回受控 422，过期 revision 返回 409，跨用户与不
/// 存在资源/配置统一返回 404。
pub async fn bind_resource(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, input_id)): Path<(i32, String)>,
    Json(body): Json<BindStudyResourceRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .bind_resource(&user, id, &input_id, body)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

/// `POST /api/study-configurations/{id}/bindings/{inputId}/mapping`：映射并绑定派生资源。
///
/// 请求携带精确 revision、稳定来源资源 ID 与受控列映射；成功发布带血缘的不可变 Parquet
/// 资源、绑定它、递增配置 revision 并返回 200。失败不改写来源资源；过期 revision 返回 409，
/// 非法映射返回受控 422，跨用户与不存在资源统一返回 404。
pub async fn map_resource(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, input_id)): Path<(i32, String)>,
    Json(body): Json<crate::application::MapStudyResourceRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .map_resource(&user, id, &input_id, body)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

/// `PUT /api/study-configurations/{id}/structs/{inputId}`：更新本次研究的 Struct 参数覆盖。
///
/// 请求携带精确 revision 与受控字段值；成功只更新配置私有覆盖、递增 revision、同步卡片并
/// 返回 200，不修改长期 Struct 资源。过期 revision 返回 409，字段/类型错误返回受控 422，
/// 跨用户与不存在配置统一返回 404。
pub async fn update_struct(
    State(state): State<AppState>,
    user: CurrentUser,
    Path((id, input_id)): Path<(i32, String)>,
    Json(body): Json<UpdateStudyStructRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .update_struct(&user, id, &input_id, body)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

/// `PUT /api/study-configurations/{id}/results`：更新有序结果选择。
///
/// 请求必须匹配当前 revision，结果需唯一且满足门面 fixed/minimum/conditional 约束；成功按
/// 请求顺序持久化、递增 revision、同步卡片并返回 200，不确认配置。过期 revision 返回 409，
/// 非法集合返回 422 `result_selection_invalid`，跨用户与不存在配置统一返回 404。
pub async fn select_results(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<SelectStudyResultsRequest>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .select_results(&user, id, body)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

/// `POST /api/study-configurations/{id}/confirm`：固定按钮确认配置并启动受管工作流。
///
/// 请求仅含精确 `expectedRevision` 与幂等 `clientOperationId`。service 在事务内重验当前模型、
/// 资源、engine snapshot 和结果合同，确认配置并创建 workflow/provisional run；handler 再同步
/// confirmed 卡片，成功或幂等重放返回 200。同一操作不同请求、过期 revision 或会话已有执行
/// 返回稳定 409，跨用户与不存在配置统一返回 404。
pub async fn confirm(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Json(body): Json<ConfirmStudyRequest>,
) -> Result<Json<ManagedWorkflowView>, ApiError> {
    let workflow = state
        .managed_workflow_service
        .confirm_configuration_and_start(&user, id, body)
        .await?;
    let configuration = state.study_configuration_service.get(&user, id).await?;
    publish(&state, &user, &configuration).await?;
    Ok(Json(workflow))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CancelQuery {
    expected_revision: i64,
}

/// `DELETE /api/study-configurations/{id}?expectedRevision=...`：取消未确认配置。
///
/// query revision 必须精确匹配；成功递增 revision、同步 cancelled 卡片并返回 200，不创建
/// workflow 或 run。过期或已终态配置返回 409 `configuration_stale`，跨用户与不存在配置
/// 统一返回 404。
pub async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Query(query): Query<CancelQuery>,
) -> Result<Json<StudyConfigurationView>, ApiError> {
    let view = state
        .study_configuration_service
        .cancel(&user, id, query.expected_revision)
        .await?;
    publish(&state, &user, &view).await?;
    Ok(Json(view))
}

async fn publish(
    state: &AppState,
    user: &CurrentUser,
    view: &StudyConfigurationView,
) -> Result<(), ApiError> {
    let action_card =
        serde_json::to_value(view).map_err(|error| ApiError::internal(error.to_string()))?;
    EventWriter::new(
        user.id.clone(),
        view.session_id,
        state.message_repo.clone(),
        state.events.clone(),
    )
    .study_config(StudyConfigCard {
        configuration_id: view.id.to_string(),
        revision: view.revision,
        state: action_card["status"]
            .as_str()
            .unwrap_or("collecting_inputs")
            .to_string(),
        action_card,
    })
    .await?;
    Ok(())
}
