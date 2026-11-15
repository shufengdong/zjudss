//! 不可变文件下载响应的内部 HTTP helper。
//!
//! 调用方先按当前用户和稳定资源 ID 解析并打开文件；本模块只读取已打开的句柄并设置下载
//! header，不解析服务器路径，也不决定资源授权或生命周期。

use axum::body::{Body, Bytes};
use axum::http::{Response, StatusCode, header};
use tokio::io::AsyncReadExt;

use super::ApiError;

const DOWNLOAD_CHUNK_BYTES: usize = 64 * 1024;

/// 把已授权的文件句柄编码为 200 attachment 响应。
///
/// 文件读取或响应构造失败统一收敛为内部错误；文件名和媒体类型由拥有领域语义的调用方提供。
pub(super) async fn response(
    file: std::fs::File,
    media_type: impl Into<String>,
    file_name: impl Into<String>,
) -> Result<Response<Body>, ApiError> {
    let size = file
        .metadata()
        .map_err(|error| ApiError::internal(error.to_string()))?;
    let source = tokio::fs::File::from_std(file);
    let stream = futures::stream::try_unfold(source, read_chunk);
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, media_type.into())
        .header(header::CONTENT_LENGTH, size.len().to_string())
        .header(
            header::CONTENT_DISPOSITION,
            content_disposition(&file_name.into()),
        )
        .body(Body::from_stream(stream))
        .map_err(|error| ApiError::internal(error.to_string()))
}

fn content_disposition(file_name: &str) -> String {
    let ascii_name = ascii_fallback_file_name(file_name);
    if ascii_name == file_name {
        return format!("attachment; filename=\"{ascii_name}\"");
    }

    format!(
        "attachment; filename=\"{ascii_name}\"; filename*=UTF-8''{}",
        encode_rfc5987(file_name)
    )
}

fn ascii_fallback_file_name(file_name: &str) -> String {
    let encoded = file_name
        .chars()
        .map(|character| match character {
            character if character.is_ascii_alphanumeric() => character,
            '-' | '_' | '.' | ' ' => character,
            _ => '_',
        })
        .collect::<String>();
    let stem = encoded
        .rsplit_once('.')
        .map_or(encoded.as_str(), |(stem, _)| stem);
    if stem
        .chars()
        .any(|character| character.is_ascii_alphanumeric())
    {
        return encoded;
    }

    let extension = file_name.rsplit_once('.').map(|(_, extension)| extension);
    match extension.filter(|extension| {
        !extension.is_empty()
            && extension.len() <= 16
            && extension.bytes().all(|byte| byte.is_ascii_alphanumeric())
    }) {
        Some(extension) => format!("download.{extension}"),
        None => "download".into(),
    }
}

fn encode_rfc5987(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'!' | b'#' | b'$' | b'&' | b'+' | b'-' | b'.' | b'^' | b'_' | b'`' | b'|' | b'~'
            )
        {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write;
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

async fn read_chunk(
    mut source: tokio::fs::File,
) -> std::io::Result<Option<(Bytes, tokio::fs::File)>> {
    let mut bytes = vec![0_u8; DOWNLOAD_CHUNK_BYTES];
    let read = source.read(&mut bytes).await?;
    if read == 0 {
        return Ok(None);
    }
    bytes.truncate(read);
    Ok(Some((Bytes::from(bytes), source)))
}

#[cfg(test)]
mod tests {
    use futures::StreamExt;

    use super::response;

    #[tokio::test]
    async fn attachment_response_streams_large_files_in_bounded_chunks() {
        let directory = tempfile::tempdir().expect("create attachment fixture");
        let path = directory.path().join("result.parquet");
        let content = vec![7_u8; 128 * 1024 + 17];
        std::fs::write(&path, &content).expect("write attachment fixture");

        let response = response(
            std::fs::File::open(path).expect("open attachment fixture"),
            "application/vnd.apache.parquet",
            "result.parquet",
        )
        .await
        .unwrap_or_else(|_| panic!("build attachment response"));

        assert_eq!(
            response.headers()[axum::http::header::CONTENT_LENGTH],
            content.len().to_string()
        );
        let mut chunks = response.into_body().into_data_stream();
        let first = chunks
            .next()
            .await
            .expect("first body chunk")
            .expect("read first body chunk");
        assert!(first.len() <= 64 * 1024);
        let mut received = first.to_vec();
        let mut chunk_count = 1;
        while let Some(chunk) = chunks.next().await {
            let chunk = chunk.expect("read attachment body chunk");
            assert!(chunk.len() <= 64 * 1024);
            received.extend_from_slice(&chunk);
            chunk_count += 1;
        }
        assert!(chunk_count > 1, "large file must use multiple chunks");
        assert_eq!(received, content);
    }

    #[tokio::test]
    async fn attachment_response_reports_an_empty_file_without_body_chunks() {
        let file = tempfile::tempfile().expect("create empty attachment fixture");
        let response = response(file, "application/octet-stream", "empty.bin")
            .await
            .unwrap_or_else(|_| panic!("build empty attachment response"));

        assert_eq!(response.headers()[axum::http::header::CONTENT_LENGTH], "0");
        let mut chunks = response.into_body().into_data_stream();
        assert!(chunks.next().await.is_none());
    }

    #[tokio::test]
    async fn attachment_response_encodes_non_ascii_and_quoted_file_names() {
        let file = tempfile::tempfile().expect("create attachment fixture");
        let response = response(file, "application/octet-stream", "更新后的\"数据.csv")
            .await
            .unwrap_or_else(|_| panic!("build attachment response"));

        assert_eq!(
            response.headers()[axum::http::header::CONTENT_DISPOSITION],
            "attachment; filename=\"download.csv\"; filename*=UTF-8''%E6%9B%B4%E6%96%B0%E5%90%8E%E7%9A%84%22%E6%95%B0%E6%8D%AE.csv"
        );
    }
}
