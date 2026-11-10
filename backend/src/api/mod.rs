//! HTTP/SSE 入口与公共路由合同。
//!
//! 本模块集中注册所有 Axum 路由、应用当前用户中间件和按端点覆盖请求体限制，并从同一份
//! 声明生成 [`route_manifest`]。`/health`、登录和登出保持公开；其余 `/api` 路由统一经过
//! [`crate::auth::require_user`]。各子模块只负责 HTTP extractor、DTO/响应映射和调用 application
//! facade，不在此层复制领域状态机、仓储事务或引擎规则。

mod analysis;
mod attachment;
mod auth;
mod chart_views;
mod comparisons;
mod data_resources;
mod datasets;
mod error;
mod events;
mod examples;
mod model_dataframes;
mod query;
mod retries;
mod runs;
mod scripts;
mod sessions;
mod storage;
mod study_configurations;
mod uploads;
mod workflows;
mod workspaces;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, patch, post, put};

use crate::application::AppState;

pub use error::ApiError;

/// `GET /health`：公开的进程存活探针。
///
/// 不读取数据库或外部引擎；进程能够处理请求时返回 200 和纯文本 `ok`。
async fn health() -> &'static str {
    "ok"
}

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
/// 一条已注册 HTTP 路由的方法与路径合同。
///
/// 该类型由生产路由声明同步生成，供合同清单与自动化测试读取，不承载 handler 行为或 DTO
/// 定义。
pub struct RouteContract {
    /// 大写 HTTP 方法，例如 `GET` 或 `POST`。
    pub method: &'static str,
    /// Axum 注册路径；路径参数保留 `{id}` 形式。
    pub path: &'static str,
}

fn build_router_and_manifest() -> (Router<AppState>, Router<AppState>, Vec<RouteContract>) {
    let mut public = Router::new();
    let mut protected = Router::new();
    let mut manifest = Vec::new();
    macro_rules! add_route {
        ($path:literal, $handler:expr, [$($method:literal),+ $(,)?]) => {{
            protected = protected.route($path, $handler);
            $(manifest.push(RouteContract { method: $method, path: $path });)+
        }};
    }
    macro_rules! add_public_route {
        ($path:literal, $handler:expr, [$($method:literal),+ $(,)?]) => {{
            public = public.route($path, $handler);
            $(manifest.push(RouteContract { method: $method, path: $path });)+
        }};
    }

    add_public_route!("/health", get(health), ["GET"]);
    add_public_route!("/api/auth/login", post(auth::login), ["POST"]);
    add_public_route!("/api/auth/logout", post(auth::logout), ["POST"]);
    add_route!("/api/me", get(auth::me), ["GET"]);
    add_route!("/api/storage-usage", get(storage::usage), ["GET"]);
    add_route!("/api/examples", get(examples::list), ["GET"]);
    add_route!("/api/examples/{id}", get(examples::get), ["GET"]);
    add_route!(
        "/api/sessions/{id}/draft/example",
        post(examples::apply),
        ["POST"]
    );
    add_route!(
        "/api/sessions",
        get(sessions::list).post(sessions::create),
        ["GET", "POST"]
    );
    add_route!(
        "/api/history/search",
        get(sessions::search_history),
        ["GET"]
    );
    add_route!(
        "/api/sessions/{id}",
        patch(sessions::rename).delete(sessions::delete),
        ["PATCH", "DELETE"]
    );
    add_route!(
        "/api/sessions/{id}/deletion-preview",
        post(sessions::deletion_preview),
        ["POST"]
    );
    add_route!(
        "/api/sessions/{id}/messages",
        get(sessions::messages).post(sessions::post_message),
        ["GET", "POST"]
    );
    add_route!(
        "/api/sessions/{id}/upload-batches",
        post(uploads::create).layer(DefaultBodyLimit::disable()),
        ["POST"]
    );
    add_route!(
        "/api/upload-batches/{id}",
        get(uploads::get).delete(uploads::discard),
        ["GET", "DELETE"]
    );
    add_route!(
        "/api/upload-batches/{id}/parse-settings",
        put(uploads::parse_settings),
        ["PUT"]
    );
    add_route!(
        "/api/upload-batches/{id}/candidates",
        put(uploads::candidates),
        ["PUT"]
    );
    add_route!(
        "/api/upload-batches/{id}/confirm",
        post(data_resources::confirm_upload),
        ["POST"]
    );
    add_route!(
        "/api/sessions/{id}/data-resources",
        get(data_resources::list),
        ["GET"]
    );
    add_route!(
        "/api/data-resources/{id}",
        get(data_resources::get)
            .patch(data_resources::update)
            .delete(data_resources::delete_resource),
        ["GET", "PATCH", "DELETE"]
    );
    add_route!(
        "/api/data-resources/{id}/rows",
        get(data_resources::rows),
        ["GET"]
    );
    add_route!(
        "/api/data-resources/{id}/download",
        get(data_resources::download),
        ["GET"]
    );
    add_route!(
        "/api/data-resources/{id}/deletion-preview",
        post(data_resources::deletion_preview),
        ["POST"]
    );
    add_route!(
        "/api/data-resources/{id}/structured-replacement-previews",
        post(data_resources::structured_preview),
        ["POST"]
    );
    add_route!(
        "/api/data-resource-replacement-previews/{id}/confirm",
        post(data_resources::confirm_structured_preview),
        ["POST"]
    );
    add_route!(
        "/api/data-resources/{id}/replacement-batches",
        post(uploads::replacement).layer(DefaultBodyLimit::disable()),
        ["POST"]
    );
    add_route!("/api/sessions/{id}/events", get(events::stream), ["GET"]);
    add_route!(
        "/api/study-configurations/{id}",
        get(study_configurations::get).delete(study_configurations::cancel),
        ["GET", "DELETE"]
    );
    add_route!(
        "/api/study-configurations/{id}/facade",
        put(study_configurations::select_facade),
        ["PUT"]
    );
    add_route!(
        "/api/study-configurations/{id}/bindings/{inputId}",
        put(study_configurations::bind_resource),
        ["PUT"]
    );
    add_route!(
        "/api/study-configurations/{id}/bindings/{inputId}/mapping",
        post(study_configurations::map_resource),
        ["POST"]
    );
    add_route!(
        "/api/study-configurations/{id}/structs/{inputId}",
        put(study_configurations::update_struct),
        ["PUT"]
    );
    add_route!(
        "/api/study-configurations/{id}/results",
        put(study_configurations::select_results),
        ["PUT"]
    );
    add_route!(
        "/api/study-configurations/{id}/confirm",
        post(study_configurations::confirm),
        ["POST"]
    );
    add_route!("/api/workflows/{id}", get(workflows::get), ["GET"]);
    add_route!(
        "/api/workflows/{id}/cancel",
        post(workflows::cancel),
        ["POST"]
    );
    add_route!(
        "/api/sessions/{id}/scripts",
        get(scripts::list)
            .post(scripts::upload)
            .layer(DefaultBodyLimit::disable()),
        ["GET", "POST"]
    );
    add_route!(
        "/api/sessions/{id}/scripts/text",
        post(scripts::create_text),
        ["POST"]
    );
    add_route!("/api/scripts/{id}", get(scripts::detail), ["GET"]);
    add_route!(
        "/api/sessions/{id}/workspace",
        get(workspaces::workspace),
        ["GET"]
    );
    add_route!(
        "/api/sessions/{id}/versions",
        get(workspaces::versions).post(workspaces::publish),
        ["GET", "POST"]
    );
    add_route!(
        "/api/versions/{id}",
        get(workspaces::version).delete(workspaces::delete_version),
        ["GET", "DELETE"]
    );
    add_route!(
        "/api/versions/{id}/activate",
        post(workspaces::activate),
        ["POST"]
    );
    add_route!(
        "/api/versions/{id}/deletion-preview",
        post(workspaces::deletion_preview),
        ["POST"]
    );
    add_route!("/api/scripts/{id}/runs", post(runs::create), ["POST"]);
    add_route!(
        "/api/scripts/{id}/retry-previews",
        post(scripts::retry_preview),
        ["POST"]
    );
    add_route!(
        "/api/retry-previews/{id}/confirm",
        post(retries::confirm),
        ["POST"]
    );
    add_route!("/api/sessions/{id}/runs", get(runs::list), ["GET"]);
    add_route!("/api/runs/{id}", get(runs::get), ["GET"]);
    add_route!(
        "/api/runs/{id}/deletion-preview",
        post(runs::deletion_preview),
        ["POST"]
    );
    add_route!("/api/runs/{id}", delete(runs::delete), ["DELETE"]);
    add_route!("/api/runs/{id}/datasets", get(runs::datasets), ["GET"]);
    add_route!(
        "/api/datasets/{id}",
        get(datasets::metadata).delete(datasets::delete),
        ["GET", "DELETE"]
    );
    add_route!("/api/datasets/{id}/rows", get(datasets::rows), ["GET"]);
    add_route!(
        "/api/datasets/{id}/download",
        get(datasets::download),
        ["GET"]
    );
    add_route!(
        "/api/datasets/{id}/chart-capabilities",
        get(chart_views::capabilities),
        ["GET"]
    );
    add_route!(
        "/api/datasets/{id}/chart-previews",
        post(chart_views::preview),
        ["POST"]
    );
    add_route!(
        "/api/datasets/{id}/views",
        post(chart_views::save),
        ["POST"]
    );
    add_route!(
        "/api/sessions/{id}/dataset-views",
        get(chart_views::list),
        ["GET"]
    );
    add_route!(
        "/api/model-dataframes/{id}",
        get(model_dataframes::version_metadata),
        ["GET"]
    );
    add_route!(
        "/api/model-dataframes/{id}/rows",
        get(model_dataframes::version_rows),
        ["GET"]
    );
    add_route!(
        "/api/model-dataframes/{id}/download",
        get(model_dataframes::version_download),
        ["GET"]
    );
    add_route!(
        "/api/draft-dataframes/{id}",
        get(model_dataframes::draft_metadata),
        ["GET"]
    );
    add_route!(
        "/api/draft-dataframes/{id}/rows",
        get(model_dataframes::draft_rows),
        ["GET"]
    );
    add_route!(
        "/api/draft-dataframes/{id}/download",
        get(model_dataframes::draft_download),
        ["GET"]
    );
    add_route!("/api/analysis-runs/{id}", get(analysis::get), ["GET"]);
    add_route!(
        "/api/dataset-views/{id}",
        get(chart_views::metadata).delete(chart_views::delete),
        ["GET", "DELETE"]
    );
    add_route!(
        "/api/dataset-views/{id}/data",
        get(chart_views::data),
        ["GET"]
    );
    add_route!("/api/run-comparisons/{id}", get(comparisons::get), ["GET"]);
    (public, protected, manifest)
}

/// 构造生产 Axum 路由器并注入应用状态。
///
/// 公开路由不要求登录；受保护路由统一应用当前用户认证中间件。上传和脚本等 multipart 端点
/// 在各自注册位置显式关闭 Axum 默认 body limit，实际容量仍由对应 application service 控制。
pub fn router(state: AppState) -> Router {
    let (public, protected, _) = build_router_and_manifest();
    public
        .merge(protected.route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_user,
        )))
        .with_state(state)
}

/// 返回与生产路由同源的 method/path 清单。
///
/// 清单只描述公开 HTTP 表面；状态码、请求/响应 DTO 和稳定错误语义由各 handler、
/// [`ApiError`] 与 `docs/api-design.md` 共同说明。
pub fn route_manifest() -> Vec<RouteContract> {
    build_router_and_manifest().2
}
