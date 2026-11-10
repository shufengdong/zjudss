//! 不可变文件下载响应的内部 HTTP helper。
//!
//! 调用方先按当前用户和稳定资源 ID 解析并打开文件；本模块只读取已打开的句柄并设置下载
//! header，不解析服务器路径，也不决定资源授权或生命周期。

use axum::body::Body;
use axum::http::{Response, StatusCode, header};
use tokio::io::AsyncReadExt;

use super::ApiError;

/// 把已授权的文件句柄编码为 200 attachment 响应。
///
/// 文件读取或响应构造失败统一收敛为内部错误；文件名和媒体类型由拥有领域语义的调用方提供。
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
