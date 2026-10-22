//! HTTP 层：axum 路由、handler、DTO 与 SSE。

mod analysis;
mod auth;
mod comparisons;
mod datasets;
mod events;
mod examples;
mod ingestions;
mod sessions;
mod storage;
mod tasks;
mod workspaces;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};

use crate::adapters::AdapterError;
use crate::application::AnalysisError;
use crate::application::AppError;
use crate::application::AppState;
use crate::application::ArtifactError;
use crate::application::ComparisonError;
use crate::application::DatasetError;
use crate::application::HistoryError;
use crate::application::IngestionError;
use crate::application::WorkspaceError;
use crate::db::repositories::RepoError;

/// 统一 API 错误：把仓储/适配器错误映射为 HTTP 响应。
pub struct ApiError {
    status: StatusCode,
    message: String,
    code: &'static str,
    details: Option<serde_json::Value>,
}

impl ApiError {
    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            message: message.into(),
            code: "internal_error",
            details: None,
        }
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            message: message.into(),
            code: "not_found",
            details: None,
        }
    }

    pub(crate) fn unauthorized(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: message.into(),
            code: "unauthorized",
            details: None,
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            axum::Json(ApiErrorBody {
                error: self.message,
                code: self.code,
                details: self.details,
            }),
        )
            .into_response()
    }
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiErrorBody {
    error: String,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::QueueFull => Self {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: error.to_string(),
                code: "agent_queue_full",
                details: None,
            },
        }
    }
}

impl From<RepoError> for ApiError {
    fn from(err: RepoError) -> Self {
        match err {
            RepoError::NotFound => ApiError::not_found("资源不存在"),
            RepoError::QuotaExceeded => Self {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: "存储配额不足".to_string(),
                code: "storage_quota_exceeded",
                details: None,
            },
            other => ApiError::internal(other.to_string()),
        }
    }
}

impl From<ArtifactError> for ApiError {
    fn from(error: ArtifactError) -> Self {
        match error {
            ArtifactError::NotFound => Self::not_found("资源不存在"),
            ArtifactError::QuotaExceeded => Self {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: "存储配额不足".to_string(),
                code: "storage_quota_exceeded",
                details: None,
            },
            ArtifactError::Storage(error) if error.code() == "artifact_too_large" => Self {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: error.to_string(),
                code: "upload_too_large",
                details: None,
            },
            other => Self::internal(other.to_string()),
        }
    }
}

impl From<IngestionError> for ApiError {
    fn from(error: IngestionError) -> Self {
        match error {
            IngestionError::NotFound => Self::not_found("资源不存在"),
            IngestionError::IdempotencyConflict => Self {
                status: StatusCode::CONFLICT,
                message: "clientBatchId 已用于不同内容".to_string(),
                code: "idempotency_conflict",
                details: None,
            },
            IngestionError::InvalidUpload(message) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message,
                code: "invalid_upload",
                details: None,
            },
            IngestionError::UploadTooLarge(message) => Self {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message,
                code: "upload_too_large",
                details: None,
            },
            IngestionError::Format(error) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message: error.to_string(),
                code: "invalid_model_archive",
                details: Some(serde_json::json!({ "reason": error.code() })),
            },
            IngestionError::Artifact(error) => error.into(),
            IngestionError::Workspace(error) => error.into(),
            IngestionError::Repository(error) => error.into(),
        }
    }
}

impl From<WorkspaceError> for ApiError {
    fn from(error: WorkspaceError) -> Self {
        match error {
            WorkspaceError::NotFound => Self::not_found("资源不存在"),
            WorkspaceError::VersionConflict => Self {
                status: StatusCode::CONFLICT,
                message: "模型版本基线已变化，请重新读取工作区".to_string(),
                code: "version_conflict",
                details: None,
            },
            WorkspaceError::InvalidManifest | WorkspaceError::InvalidState(_) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message: error.to_string(),
                code: "invalid_model_version",
                details: None,
            },
            WorkspaceError::Repository(other) => Self::internal(other.to_string()),
        }
    }
}

impl From<AdapterError> for ApiError {
    fn from(err: AdapterError) -> Self {
        ApiError::internal(err.to_string())
    }
}

impl From<DatasetError> for ApiError {
    fn from(error: DatasetError) -> Self {
        match error {
            DatasetError::NotFound => Self::not_found("资源不存在"),
            DatasetError::InvalidCursor | DatasetError::InvalidPage(_) => Self {
                status: StatusCode::BAD_REQUEST,
                message: error.to_string(),
                code: "invalid_dataset_page",
                details: None,
            },
            other => Self::internal(other.to_string()),
        }
    }
}
impl From<AnalysisError> for ApiError {
    fn from(error: AnalysisError) -> Self {
        match error.code {
            "analysis_not_found" | "dataset_not_found" => Self::not_found("资源不存在"),
            code => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message: error.message,
                code,
                details: error.details,
            },
        }
    }
}
impl From<ComparisonError> for ApiError {
    fn from(error: ComparisonError) -> Self {
        match error.code {
            "not_found" => Self::not_found("资源不存在"),
            code => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message: error.message,
                code,
                details: None,
            },
        }
    }
}
impl From<HistoryError> for ApiError {
    fn from(error: HistoryError) -> Self {
        match error.code {
            "not_found" => Self::not_found("资源不存在"),
            "history_search_unavailable" => Self {
                status: StatusCode::SERVICE_UNAVAILABLE,
                message: error.message,
                code: error.code,
                details: None,
            },
            _ => Self {
                status: StatusCode::BAD_REQUEST,
                message: error.message,
                code: error.code,
                details: None,
            },
        }
    }
}

async fn health() -> &'static str {
    "ok"
}

#[derive(Debug, Clone, Copy, serde::Serialize, PartialEq, Eq)]
pub struct RouteContract {
    pub method: &'static str,
    pub path: &'static str,
}

fn build_router_and_manifest() -> (Router<AppState>, Vec<RouteContract>) {
    let mut router = Router::new();
    let mut manifest = Vec::new();
    macro_rules! add_route {
        ($path:literal, $handler:expr, [$($method:literal),+ $(,)?]) => {{
            router = router.route($path, $handler);
            $(manifest.push(RouteContract { method: $method, path: $path });)+
        }};
    }

    add_route!("/health", get(health), ["GET"]);
    add_route!("/api/me", get(auth::me), ["GET"]);
    add_route!("/api/storage-usage", get(storage::usage), ["GET"]);
    add_route!("/api/examples", get(examples::list), ["GET"]);
    add_route!("/api/examples/{id}", get(examples::get), ["GET"]);
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
        "/api/sessions/{id}/messages",
        get(sessions::messages).post(sessions::post_message),
        ["GET", "POST"]
    );
    add_route!(
        "/api/sessions/{id}/attachments",
        post(sessions::upload_attachment).layer(DefaultBodyLimit::disable()),
        ["POST"]
    );
    add_route!("/api/sessions/{id}/events", get(events::stream), ["GET"]);
    add_route!("/api/ingestions/{id}", get(ingestions::get), ["GET"]);
    add_route!(
        "/api/sessions/{id}/workspace",
        get(workspaces::workspace),
        ["GET"]
    );
    add_route!(
        "/api/sessions/{id}/versions",
        get(workspaces::versions),
        ["GET"]
    );
    add_route!("/api/versions/{id}", get(workspaces::version), ["GET"]);
    add_route!("/api/tasks/{id}", get(tasks::get), ["GET"]);
    add_route!(
        "/api/sessions/{id}/tasks",
        get(tasks::list_session),
        ["GET"]
    );
    add_route!("/api/tasks/{id}/datasets", get(tasks::datasets), ["GET"]);
    add_route!("/api/datasets/{id}", get(datasets::metadata), ["GET"]);
    add_route!("/api/datasets/{id}/rows", get(datasets::rows), ["GET"]);
    add_route!(
        "/api/datasets/{id}/download",
        get(datasets::download),
        ["GET"]
    );
    add_route!("/api/analysis-runs/{id}", get(analysis::get), ["GET"]);
    add_route!(
        "/api/dataset-views/{id}/data",
        get(analysis::view_data),
        ["GET"]
    );
    add_route!("/api/task-comparisons/{id}", get(comparisons::get), ["GET"]);
    (router, manifest)
}

pub fn router() -> Router<AppState> {
    build_router_and_manifest().0
}

pub fn route_manifest() -> Vec<RouteContract> {
    build_router_and_manifest().1
}
