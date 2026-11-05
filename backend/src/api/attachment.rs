use axum::body::Body;
use axum::http::{Response, StatusCode, header};
use tokio::io::AsyncReadExt;

use super::ApiError;

pub(super) async fn response(
    file: std::fs::File,
    media_type: impl Into<String>,
    file_name: impl Into<String>,
) -> Result<Response<Body>, ApiError> {
    let mut source = tokio::fs::File::from_std(file);
    let mut bytes = Vec::new();
    source
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| ApiError::internal(error.to_string()))?;
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, media_type.into())
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename=\"{}\"", file_name.into()),
        )
        .body(Body::from(bytes))
        .map_err(|error| ApiError::internal(error.to_string()))
}
