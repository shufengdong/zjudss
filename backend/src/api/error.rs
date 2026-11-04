use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};

use crate::adapters::AdapterError;
use crate::application::AnalysisError;
use crate::application::AppError;
use crate::application::ArtifactError;
use crate::application::CaseImportError;
use crate::application::ComparisonError;
use crate::application::DataResourceError;
use crate::application::DatasetError;
use crate::application::HistoryError;
use crate::application::ModelDataFrameReadError;
use crate::application::RunError;
use crate::application::ScriptError;
use crate::application::StudyConfigurationError;
use crate::application::UploadBatchError;
use crate::application::VersionError;
use crate::application::WorkflowError;
use crate::application::charting::ChartServiceError;
use crate::db::repositories::RepoError;

/// 统一 API 错误：把仓储/适配器错误映射为 HTTP 响应。
pub struct ApiError {
    status: StatusCode,
    message: String,
    code: &'static str,
    details: Option<serde_json::Value>,
}

impl From<WorkflowError> for ApiError {
    fn from(error: WorkflowError) -> Self {
        let code = error.code();
        let status = match code {
            "not_found" => StatusCode::NOT_FOUND,
            "workflow_in_progress"
            | "configuration_stale"
            | "idempotency_conflict"
            | "workflow_terminal_conflict" => StatusCode::CONFLICT,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        let response = Self::public(status, code, error.to_string());
        if let Some(id) = error.workflow_id() {
            response.with_details(serde_json::json!({"workflowId":id.to_string()}))
        } else {
            response
        }
    }
}

impl ApiError {
    pub(crate) fn public(
        status: StatusCode,
        code: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            status,
            message: message.into(),
            code,
            details: None,
        }
    }

    pub(crate) fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }

    pub(crate) fn internal(message: impl Into<String>) -> Self {
        Self::public(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", message)
    }

    pub(crate) fn not_found(message: impl Into<String>) -> Self {
        Self::public(StatusCode::NOT_FOUND, "not_found", message)
    }

    pub(crate) fn authentication_required() -> Self {
        Self::public(
            StatusCode::UNAUTHORIZED,
            "authentication_required",
            "请先登录",
        )
    }

    pub(crate) fn invalid_credentials() -> Self {
        Self::public(
            StatusCode::UNAUTHORIZED,
            "invalid_credentials",
            "用户 ID 或密码错误",
        )
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
struct ApiErrorBody {
    error: String,
    code: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    details: Option<serde_json::Value>,
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        match error {
            AppError::QueueFull => Self::public(
                StatusCode::TOO_MANY_REQUESTS,
                "agent_queue_full",
                error.to_string(),
            ),
        }
    }
}

impl From<RepoError> for ApiError {
    fn from(err: RepoError) -> Self {
        match err {
            RepoError::NotFound => ApiError::not_found("资源不存在"),
            RepoError::QuotaExceeded => Self::public(
                StatusCode::PAYLOAD_TOO_LARGE,
                "storage_quota_exceeded",
                "存储配额不足".to_string(),
            ),
            RepoError::InvalidState(code)
                if matches!(
                    code.as_str(),
                    "deletion_impact_changed"
                        | "invalid_deletion_impact_token"
                        | "session_deleting"
                ) =>
            {
                let public_code = match code.as_str() {
                    "deletion_impact_changed" => "deletion_impact_changed",
                    "invalid_deletion_impact_token" => "invalid_deletion_impact_token",
                    _ => "session_deleting",
                };
                Self::public(StatusCode::CONFLICT, public_code, code)
            }
            other => ApiError::internal(other.to_string()),
        }
    }
}

impl From<ArtifactError> for ApiError {
    fn from(error: ArtifactError) -> Self {
        match error {
            ArtifactError::NotFound => Self::not_found("资源不存在"),
            ArtifactError::QuotaExceeded => Self::public(
                StatusCode::PAYLOAD_TOO_LARGE,
                "storage_quota_exceeded",
                "存储配额不足".to_string(),
            ),
            ArtifactError::Storage(error) if error.code() == "artifact_too_large" => Self::public(
                StatusCode::PAYLOAD_TOO_LARGE,
                "upload_too_large",
                error.to_string(),
            ),
            other => Self::internal(other.to_string()),
        }
    }
}

impl From<ChartServiceError> for ApiError {
    fn from(error: ChartServiceError) -> Self {
        let code = error.code();
        let status = match code {
            "not_found" => StatusCode::NOT_FOUND,
            "chart_name_conflict"
            | "chart_operation_conflict"
            | "chart_operation_gone"
            | "chart_source_deleted" => StatusCode::CONFLICT,
            "chart_too_large" | "chart_capabilities_too_large" | "storage_quota_exceeded" => {
                StatusCode::PAYLOAD_TOO_LARGE
            }
            "chart_materialization_failed" => StatusCode::INTERNAL_SERVER_ERROR,
            _ => StatusCode::BAD_REQUEST,
        };
        let details = error.details();
        let response = Self::public(status, code, error.to_string());
        if let Some(details) = details {
            response.with_details(details)
        } else {
            response
        }
    }
}

impl From<UploadBatchError> for ApiError {
    fn from(error: UploadBatchError) -> Self {
        let status = match error.code() {
            "not_found" => StatusCode::NOT_FOUND,
            "idempotency_conflict" | "upload_batch_expired" | "upload_state_invalid" => {
                StatusCode::CONFLICT
            }
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
            "upload_state_invalid" => "upload_state_invalid",
            "upload_file_too_large" => "upload_file_too_large",
            "upload_batch_too_large" => "upload_batch_too_large",
            "archive_limits_exceeded" => "archive_limits_exceeded",
            "storage_quota_exceeded" => "storage_quota_exceeded",
            "excel_formula_value_missing" => "excel_formula_value_missing",
            _ => "upload_invalid",
        };
        Self::public(status, code, error.to_string())
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
        Self::public(status, code, error.to_string())
    }
}

impl From<StudyConfigurationError> for ApiError {
    fn from(error: StudyConfigurationError) -> Self {
        let code = error.code();
        let status = match code {
            "not_found" => StatusCode::NOT_FOUND,
            "configuration_stale" => StatusCode::CONFLICT,
            _ => StatusCode::UNPROCESSABLE_ENTITY,
        };
        Self::public(status, code, error.to_string())
    }
}

impl From<ScriptError> for ApiError {
    fn from(error: ScriptError) -> Self {
        match error {
            ScriptError::NotFound => Self::not_found("资源不存在"),
            ScriptError::Invalid(message) => {
                Self::public(StatusCode::UNPROCESSABLE_ENTITY, "invalid_script", message)
            }
            ScriptError::Artifact(error) => error.into(),
            ScriptError::Repository(error) => Self::internal(error.to_string()),
            ScriptError::Io(error) => Self::internal(error.to_string()),
            ScriptError::StudyPlan { code, message } => {
                Self::public(StatusCode::UNPROCESSABLE_ENTITY, code, message)
            }
        }
    }
}

impl From<RunError> for ApiError {
    fn from(error: RunError) -> Self {
        match error {
            RunError::NotFound => Self::not_found("资源不存在"),
            RunError::ActiveModelChanged => Self::public(
                StatusCode::CONFLICT,
                "active_model_changed",
                error.to_string(),
            ),
            RunError::ScriptStaticInvalid => Self::public(
                StatusCode::UNPROCESSABLE_ENTITY,
                "script_static_invalid",
                error.to_string(),
            ),
            RunError::ScriptIncompatible(missing) => Self::public(
                StatusCode::UNPROCESSABLE_ENTITY,
                "script_model_incompatible",
                "脚本与当前模型不兼容",
            )
            .with_details(serde_json::json!({"missingAliases":missing})),
            RunError::ConfigurationStale => Self::public(
                StatusCode::CONFLICT,
                "configuration_stale",
                error.to_string(),
            ),
            RunError::IdempotencyConflict => Self::public(
                StatusCode::CONFLICT,
                "idempotency_conflict",
                error.to_string(),
            ),
            RunError::QueueFull => Self::public(
                StatusCode::TOO_MANY_REQUESTS,
                "run_queue_full",
                error.to_string(),
            ),
            RunError::Invalid(message) => {
                Self::public(StatusCode::UNPROCESSABLE_ENTITY, "invalid_run", message)
            }
            RunError::ResultContract(code, message) => {
                Self::public(StatusCode::UNPROCESSABLE_ENTITY, code, message)
            }
            RunError::RunBusy => Self::public(StatusCode::CONFLICT, "run_busy", error.to_string()),
            RunError::ResultBusy => {
                Self::public(StatusCode::CONFLICT, "result_busy", error.to_string())
            }
            RunError::InvalidDeletionImpactToken => Self::public(
                StatusCode::CONFLICT,
                "invalid_deletion_impact_token",
                error.to_string(),
            ),
            RunError::DeletionImpactChanged => Self::public(
                StatusCode::CONFLICT,
                "deletion_impact_changed",
                error.to_string(),
            ),
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
            CaseImportError::SnapshotMismatch => Self::public(
                StatusCode::CONFLICT,
                "engine_resource_snapshot_mismatch",
                error.to_string(),
            ),
            CaseImportError::NotModelizable | CaseImportError::SourceChanged => Self::public(
                StatusCode::UNPROCESSABLE_ENTITY,
                "case_not_modelizable",
                error.to_string(),
            ),
            CaseImportError::Upload(error) => error.into(),
        }
    }
}

impl From<VersionError> for ApiError {
    fn from(error: VersionError) -> Self {
        match error {
            VersionError::NotFound => Self::not_found("资源不存在"),
            VersionError::DraftRevisionConflict => Self::public(
                StatusCode::CONFLICT,
                "draft_revision_conflict",
                error.to_string(),
            ),
            VersionError::DraftRequired | VersionError::EmptyDraft | VersionError::Invalid(_) => {
                Self::public(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "invalid_model_version",
                    error.to_string(),
                )
            }
            VersionError::DraftDiscardConfirmationRequired => Self::public(
                StatusCode::CONFLICT,
                "draft_discard_confirmation_required",
                error.to_string(),
            ),
            VersionError::ActiveVersion => Self::public(
                StatusCode::CONFLICT,
                "active_version_cannot_be_deleted",
                error.to_string(),
            ),
            VersionError::Busy => Self::public(
                StatusCode::CONFLICT,
                "version_has_active_consumers",
                error.to_string(),
            ),
            VersionError::ImpactChanged => Self::public(
                StatusCode::CONFLICT,
                "deletion_impact_changed",
                error.to_string(),
            ),
            VersionError::InvalidImpactToken => Self::public(
                StatusCode::CONFLICT,
                "invalid_deletion_impact_token",
                error.to_string(),
            ),
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
            DatasetError::InvalidCursor | DatasetError::InvalidPage(_) => Self::public(
                StatusCode::BAD_REQUEST,
                "invalid_dataset_page",
                error.to_string(),
            ),
            DatasetError::Busy => {
                Self::public(StatusCode::CONFLICT, "result_busy", error.to_string())
            }
            other => Self::internal(other.to_string()),
        }
    }
}
impl From<ModelDataFrameReadError> for ApiError {
    fn from(error: ModelDataFrameReadError) -> Self {
        match error {
            ModelDataFrameReadError::NotFound => Self::not_found("资源不存在"),
            ModelDataFrameReadError::DraftRevisionConflict => Self::public(
                StatusCode::CONFLICT,
                "draft_revision_conflict",
                error.to_string(),
            ),
            ModelDataFrameReadError::ModelScopeUnavailable => Self::public(
                StatusCode::CONFLICT,
                "model_scope_unavailable",
                error.to_string(),
            ),
            ModelDataFrameReadError::TableNotFound => Self::not_found("表不存在"),
            ModelDataFrameReadError::InvalidModelQuery(_) => Self::public(
                StatusCode::BAD_REQUEST,
                "invalid_model_query",
                error.to_string(),
            ),
            ModelDataFrameReadError::ModelQueryOutputTooLarge => Self::public(
                StatusCode::PAYLOAD_TOO_LARGE,
                "model_query_output_too_large",
                error.to_string(),
            ),
            ModelDataFrameReadError::InvalidPage(_) => {
                Self::public(StatusCode::BAD_REQUEST, "invalid_page", error.to_string())
            }
            ModelDataFrameReadError::ReadFailed(_) | ModelDataFrameReadError::Repository(_) => {
                Self::public(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "dataframe_read_failed",
                    error.to_string(),
                )
            }
        }
    }
}
impl From<AnalysisError> for ApiError {
    fn from(error: AnalysisError) -> Self {
        match error.code {
            "analysis_not_found" | "dataset_not_found" => Self::not_found("资源不存在"),
            code => {
                let response = Self::public(StatusCode::UNPROCESSABLE_ENTITY, code, error.message);
                if let Some(details) = error.details {
                    response.with_details(details)
                } else {
                    response
                }
            }
        }
    }
}
impl From<ComparisonError> for ApiError {
    fn from(error: ComparisonError) -> Self {
        match error.code {
            "not_found" => Self::not_found("资源不存在"),
            code => Self::public(StatusCode::UNPROCESSABLE_ENTITY, code, error.message),
        }
    }
}
impl From<HistoryError> for ApiError {
    fn from(error: HistoryError) -> Self {
        match error.code {
            "not_found" => Self::not_found("资源不存在"),
            "history_search_unavailable" => {
                Self::public(StatusCode::SERVICE_UNAVAILABLE, error.code, error.message)
            }
            _ => Self::public(StatusCode::BAD_REQUEST, error.code, error.message),
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::to_bytes;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use serde_json::Value;

    use super::ApiError;
    use crate::application::charting::{ChartError, ChartServiceError};
    use crate::application::{AppError, HistoryError, RunError};
    use crate::db::repositories::RepoError;

    #[tokio::test]
    async fn representative_domain_errors_keep_the_public_response_contract() {
        let cases = [
            (
                ApiError::authentication_required(),
                StatusCode::UNAUTHORIZED,
                "authentication_required",
                "请先登录",
                None,
            ),
            (
                HistoryError {
                    code: "history_query_invalid",
                    message: "invalid query".into(),
                }
                .into(),
                StatusCode::BAD_REQUEST,
                "history_query_invalid",
                "invalid query",
                None,
            ),
            (
                ApiError::not_found("missing"),
                StatusCode::NOT_FOUND,
                "not_found",
                "missing",
                None,
            ),
            (
                RunError::ConfigurationStale.into(),
                StatusCode::CONFLICT,
                "configuration_stale",
                "冻结输入的来源已经变化",
                None,
            ),
            (
                RepoError::QuotaExceeded.into(),
                StatusCode::PAYLOAD_TOO_LARGE,
                "storage_quota_exceeded",
                "存储配额不足",
                None,
            ),
            (
                RunError::Invalid("invalid".into()).into(),
                StatusCode::UNPROCESSABLE_ENTITY,
                "invalid_run",
                "invalid",
                None,
            ),
            (
                AppError::QueueFull.into(),
                StatusCode::TOO_MANY_REQUESTS,
                "agent_queue_full",
                "Agent 请求队列已满，请稍后重试",
                None,
            ),
            (
                ApiError::internal("failed"),
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "failed",
                None,
            ),
            (
                HistoryError {
                    code: "history_search_unavailable",
                    message: "unavailable".into(),
                }
                .into(),
                StatusCode::SERVICE_UNAVAILABLE,
                "history_search_unavailable",
                "unavailable",
                None,
            ),
        ];

        for (error, expected_status, expected_code, expected_message, expected_details) in cases {
            let response = error.into_response();
            assert_eq!(response.status(), expected_status);
            let body = to_bytes(response.into_body(), 16 * 1024)
                .await
                .expect("read API error response");
            let body: Value = serde_json::from_slice(&body).expect("parse API error response");
            let mut expected = serde_json::json!({
                "error": expected_message,
                "code": expected_code,
            });
            if let Some(details) = expected_details {
                expected["details"] = details;
            }
            assert_eq!(body, expected);
        }
    }

    #[tokio::test]
    async fn structured_domain_details_are_preserved() {
        let run: ApiError = RunError::ScriptIncompatible(vec!["bus".into(), "load".into()]).into();
        assert_response_details(run, serde_json::json!({"missingAliases":["bus", "load"]})).await;

        let chart_details = serde_json::json!({"column":"voltage","reason":"not_numeric"});
        let chart: ApiError = ChartServiceError::Chart(ChartError {
            code: "chart_column_invalid",
            message: "invalid column".into(),
            details: Some(chart_details.clone()),
        })
        .into();
        assert_response_details(chart, chart_details).await;
    }

    async fn assert_response_details(error: ApiError, expected: Value) {
        let response = error.into_response();
        let body = to_bytes(response.into_body(), 16 * 1024)
            .await
            .expect("read API error response");
        let body: Value = serde_json::from_slice(&body).expect("parse API error response");
        assert_eq!(body["details"], expected);
    }
}
