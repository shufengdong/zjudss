//! HTTP 层：axum 路由、handler、DTO 与 SSE。

mod events;
mod sessions;
mod tasks;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};

use crate::adapters::AdapterError;
use crate::application::AppState;
use crate::db::repo::RepoError;

/// 统一 API 错误：把仓储/适配器错误映射为 HTTP 响应。
pub struct ApiError {
    status: StatusCode,
    message: String,
}

impl ApiError {
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
        }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            axum::Json(serde_json::json!({ "error": self.message })),
        )
            .into_response()
    }
}

impl From<RepoError> for ApiError {
    fn from(err: RepoError) -> Self {
        match err {
            RepoError::NotFound => ApiError::not_found("资源不存在"),
            other => ApiError::internal(other.to_string()),
        }
    }
}

impl From<AdapterError> for ApiError {
    fn from(err: AdapterError) -> Self {
        ApiError::internal(err.to_string())
    }
}

async fn health() -> &'static str {
    "ok"
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/health", get(health))
        .route(
            "/api/sessions",
            get(sessions::list).post(sessions::create),
        )
        .route(
            "/api/sessions/{id}",
            patch(sessions::rename).delete(sessions::delete),
        )
        .route(
            "/api/sessions/{id}/messages",
            get(sessions::messages).post(sessions::post_message),
        )
        .route(
            "/api/sessions/{id}/attachments",
            post(sessions::upload_attachment).layer(DefaultBodyLimit::max(50 * 1024 * 1024)),
        )
        .route("/api/sessions/{id}/events", get(events::stream))
        .route("/api/tasks/{id}/curves", get(tasks::curves))
        .route("/api/tasks/{id}/report", get(tasks::report))
}
