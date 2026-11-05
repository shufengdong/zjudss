//! HTTP 层：axum 路由、handler、DTO 与 SSE。

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

async fn health() -> &'static str {
    "ok"
}

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
pub struct RouteContract {
    pub method: &'static str,
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

pub fn router(state: AppState) -> Router {
    let (public, protected, _) = build_router_and_manifest();
    public
        .merge(protected.route_layer(middleware::from_fn_with_state(
            state.clone(),
            crate::auth::require_user,
        )))
        .with_state(state)
}

pub fn route_manifest() -> Vec<RouteContract> {
    build_router_and_manifest().2
}
