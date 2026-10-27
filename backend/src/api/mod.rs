//! HTTP 层：axum 路由、handler、DTO 与 SSE。

mod analysis;
mod auth;
mod comparisons;
mod data_resources;
mod datasets;
mod events;
mod examples;
mod model_dataframes;
mod runs;
mod scripts;
mod sessions;
mod storage;
mod uploads;
mod workspaces;

use axum::Router;
use axum::extract::DefaultBodyLimit;
use axum::http::StatusCode;
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post, put};

use crate::adapters::AdapterError;
use crate::application::AnalysisError;
use crate::application::AppError;
use crate::application::AppState;
use crate::application::ArtifactError;
use crate::application::CaseImportError;
use crate::application::ComparisonError;
use crate::application::DataResourceError;
use crate::application::DatasetError;
use crate::application::HistoryError;
use crate::application::ModelDataFrameReadError;
use crate::application::RunError;
use crate::application::ScriptError;
use crate::application::UploadBatchError;
use crate::application::VersionError;
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

    pub(crate) fn authentication_required() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "请先登录".to_string(),
            code: "authentication_required",
            details: None,
        }
    }

    pub(crate) fn invalid_credentials() -> Self {
        Self {
            status: StatusCode::UNAUTHORIZED,
            message: "用户 ID 或密码错误".to_string(),
            code: "invalid_credentials",
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

impl From<UploadBatchError> for ApiError {
    fn from(error: UploadBatchError) -> Self {
        let status = match error.code() {
            "not_found" => StatusCode::NOT_FOUND,
            "idempotency_conflict" | "upload_batch_expired" => StatusCode::CONFLICT,
            "upload_file_too_large"
            | "upload_batch_too_large"
            | "archive_limits_exceeded"
            | "storage_quota_exceeded" => StatusCode::PAYLOAD_TOO_LARGE,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        let code = match error.code() {
            "not_found" => "not_found",
            "idempotency_conflict" => "idempotency_conflict",
            "upload_batch_expired" => "upload_batch_expired",
            "upload_file_too_large" => "upload_file_too_large",
            "upload_batch_too_large" => "upload_batch_too_large",
            "archive_limits_exceeded" => "archive_limits_exceeded",
            "storage_quota_exceeded" => "storage_quota_exceeded",
            "excel_formula_value_missing" => "excel_formula_value_missing",
            _ => "upload_invalid",
        };
        Self {
            status,
            message: error.to_string(),
            code,
            details: None,
        }
    }
}

impl From<DataResourceError> for ApiError {
    fn from(error: DataResourceError) -> Self {
        let code = error.code();
        let status = match code {
            "not_found" => StatusCode::NOT_FOUND,
            "storage_quota_exceeded" => StatusCode::PAYLOAD_TOO_LARGE,
            "resource_alias_conflict"
            | "resource_stale"
            | "draft_revision_conflict"
            | "idempotency_conflict"
            | "deletion_impact_changed"
            | "invalid_deletion_impact_token" => StatusCode::CONFLICT,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        Self {
            status,
            message: error.to_string(),
            code,
            details: None,
        }
    }
}

impl From<ScriptError> for ApiError {
    fn from(error: ScriptError) -> Self {
        match error {
            ScriptError::NotFound => Self::not_found("资源不存在"),
            ScriptError::Invalid(message) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message,
                code: "invalid_script",
                details: None,
            },
            ScriptError::Artifact(error) => error.into(),
            ScriptError::Repository(error) => Self::internal(error.to_string()),
            ScriptError::Io(error) => Self::internal(error.to_string()),
            ScriptError::StudyPlan { code, message } => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message,
                code,
                details: None,
            },
        }
    }
}

impl From<RunError> for ApiError {
    fn from(error: RunError) -> Self {
        match error {
            RunError::NotFound => Self::not_found("资源不存在"),
            RunError::ActiveModelChanged => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "active_model_changed",
                details: None,
            },
            RunError::ScriptStaticInvalid => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message: error.to_string(),
                code: "script_static_invalid",
                details: None,
            },
            RunError::ScriptIncompatible(missing) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message: "脚本与当前模型不兼容".into(),
                code: "script_model_incompatible",
                details: Some(serde_json::json!({"missingAliases":missing})),
            },
            RunError::IdempotencyConflict => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "idempotency_conflict",
                details: None,
            },
            RunError::QueueFull => Self {
                status: StatusCode::TOO_MANY_REQUESTS,
                message: error.to_string(),
                code: "run_queue_full",
                details: None,
            },
            RunError::Invalid(message) => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message,
                code: "invalid_run",
                details: None,
            },
            RunError::Artifact(error) => error.into(),
            RunError::Adapter(error) => Self::internal(error.to_string()),
            RunError::Repository(error) => Self::internal(error.to_string()),
            RunError::Io(error) => Self::internal(error.to_string()),
        }
    }
}

impl From<CaseImportError> for ApiError {
    fn from(error: CaseImportError) -> Self {
        match error {
            CaseImportError::NotFound => Self::not_found("资源不存在"),
            CaseImportError::SnapshotMismatch => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "engine_resource_snapshot_mismatch",
                details: None,
            },
            CaseImportError::NotModelizable | CaseImportError::SourceChanged => Self {
                status: StatusCode::UNPROCESSABLE_ENTITY,
                message: error.to_string(),
                code: "case_not_modelizable",
                details: None,
            },
            CaseImportError::Upload(error) => error.into(),
        }
    }
}

impl From<VersionError> for ApiError {
    fn from(error: VersionError) -> Self {
        match error {
            VersionError::NotFound => Self::not_found("资源不存在"),
            VersionError::DraftRevisionConflict => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "draft_revision_conflict",
                details: None,
            },
            VersionError::DraftRequired | VersionError::EmptyDraft | VersionError::Invalid(_) => {
                Self {
                    status: StatusCode::UNPROCESSABLE_ENTITY,
                    message: error.to_string(),
                    code: "invalid_model_version",
                    details: None,
                }
            }
            VersionError::DraftDiscardConfirmationRequired => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "draft_discard_confirmation_required",
                details: None,
            },
            VersionError::ActiveVersion => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "active_version_cannot_be_deleted",
                details: None,
            },
            VersionError::Busy => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "version_has_active_consumers",
                details: None,
            },
            VersionError::ImpactChanged => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "deletion_impact_changed",
                details: None,
            },
            VersionError::InvalidImpactToken => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "invalid_deletion_impact_token",
                details: None,
            },
            VersionError::Artifact(error) => error.into(),
            VersionError::Repository(error) => Self::internal(error.to_string()),
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
            DatasetError::Busy => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "result_busy",
                details: None,
            },
            other => Self::internal(other.to_string()),
        }
    }
}
impl From<ModelDataFrameReadError> for ApiError {
    fn from(error: ModelDataFrameReadError) -> Self {
        match error {
            ModelDataFrameReadError::NotFound => Self::not_found("资源不存在"),
            ModelDataFrameReadError::DraftRevisionConflict => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "draft_revision_conflict",
                details: None,
            },
            ModelDataFrameReadError::ModelScopeUnavailable => Self {
                status: StatusCode::CONFLICT,
                message: error.to_string(),
                code: "model_scope_unavailable",
                details: None,
            },
            ModelDataFrameReadError::TableNotFound => Self::not_found("表不存在"),
            ModelDataFrameReadError::InvalidModelQuery(_) => Self {
                status: StatusCode::BAD_REQUEST,
                message: error.to_string(),
                code: "invalid_model_query",
                details: None,
            },
            ModelDataFrameReadError::ModelQueryOutputTooLarge => Self {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                message: error.to_string(),
                code: "model_query_output_too_large",
                details: None,
            },
            ModelDataFrameReadError::InvalidPage(_) => Self {
                status: StatusCode::BAD_REQUEST,
                message: error.to_string(),
                code: "invalid_page",
                details: None,
            },
            ModelDataFrameReadError::ReadFailed(_) | ModelDataFrameReadError::Repository(_) => {
                Self {
                    status: StatusCode::INTERNAL_SERVER_ERROR,
                    message: error.to_string(),
                    code: "dataframe_read_failed",
                    details: None,
                }
            }
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
    add_route!("/api/sessions/{id}/runs", get(runs::list), ["GET"]);
    add_route!("/api/runs/{id}", get(runs::get), ["GET"]);
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
        "/api/dataset-views/{id}/data",
        get(analysis::view_data),
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
