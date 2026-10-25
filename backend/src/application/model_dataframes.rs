use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::application::dataframe_reader::{
    DataFrameColumn, DataFrameReadError, DataFrameReadSource, ParquetDataFrameReader,
};
use crate::auth::CurrentUser;
use crate::db::repositories::{
    DraftDataFrameReadRow, RepoError, VersionDataFrameReadRow, VersionRepo, VersionRepoError,
};

use super::ArtifactService;

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelDataFrameScope {
    ModelVersion,
    Draft,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDataFrameMetadata {
    pub id: String,
    pub scope: ModelDataFrameScope,
    pub owner_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
    pub table_name: String,
    pub row_count: i64,
    pub size_bytes: i64,
    pub columns: Vec<DataFrameColumn>,
    pub schema_hash: String,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDataFramePage {
    pub resource_id: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub next_cursor: Option<String>,
    pub diagnostics: BTreeMap<String, u64>,
}

#[derive(Debug, Clone, Copy)]
pub struct DraftReadExpectation {
    pub draft_id: i32,
    pub expected_revision: i64,
}

pub struct ModelDataFrameDownload {
    pub path: PathBuf,
    pub file_name: String,
    pub media_type: &'static str,
}

#[derive(Debug)]
pub enum ModelDataFrameReadError {
    NotFound,
    DraftRevisionConflict,
    InvalidPage(String),
    ReadFailed(String),
    Repository(RepoError),
}

impl std::fmt::Display for ModelDataFrameReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "资源不存在"),
            Self::DraftRevisionConflict => write!(formatter, "草稿已变化，请刷新后重试"),
            Self::InvalidPage(message) => write!(formatter, "分页参数无效: {message}"),
            Self::ReadFailed(message) => write!(formatter, "DataFrame 读取失败: {message}"),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ModelDataFrameReadError {}

impl From<DataFrameReadError> for ModelDataFrameReadError {
    fn from(error: DataFrameReadError) -> Self {
        match error {
            DataFrameReadError::NotFound => Self::NotFound,
            DataFrameReadError::InvalidCursor => Self::InvalidPage("cursor 无效".into()),
            DataFrameReadError::InvalidPage(message) => Self::InvalidPage(message),
            DataFrameReadError::Storage(message) => Self::ReadFailed(message),
        }
    }
}

fn map_repository(error: VersionRepoError) -> ModelDataFrameReadError {
    match error {
        VersionRepoError::Repository(RepoError::NotFound) => ModelDataFrameReadError::NotFound,
        VersionRepoError::DraftConflict => ModelDataFrameReadError::DraftRevisionConflict,
        VersionRepoError::Repository(error) => ModelDataFrameReadError::Repository(error),
        other => ModelDataFrameReadError::ReadFailed(format!("版本状态错误: {other:?}")),
    }
}

#[derive(Clone)]
pub struct ModelDataFrameReadService {
    versions: VersionRepo,
    reader: ParquetDataFrameReader,
}

impl ModelDataFrameReadService {
    pub fn new(versions: VersionRepo, artifacts: ArtifactService) -> Self {
        Self {
            versions,
            reader: ParquetDataFrameReader::new(artifacts),
        }
    }

    pub async fn version_metadata(
        &self,
        user: &CurrentUser,
        dataframe_id: i32,
    ) -> Result<ModelDataFrameMetadata, ModelDataFrameReadError> {
        let row = self
            .versions
            .find_version_dataframe(&user.id, dataframe_id)
            .await
            .map_err(map_repository)?;
        version_metadata(row)
    }

    pub async fn version_page(
        &self,
        user: &CurrentUser,
        dataframe_id: i32,
        limit: Option<usize>,
        cursor: Option<&str>,
        columns: &[String],
    ) -> Result<ModelDataFramePage, ModelDataFrameReadError> {
        let row = self
            .versions
            .find_version_dataframe(&user.id, dataframe_id)
            .await
            .map_err(map_repository)?;
        let page = self
            .reader
            .page(user, &version_source(&row), limit, cursor, columns)
            .await?;
        Ok(ModelDataFramePage {
            resource_id: dataframe_id.to_string(),
            columns: page.columns,
            rows: page.rows,
            next_cursor: page.next_cursor,
            diagnostics: page.diagnostics,
        })
    }

    pub async fn version_download(
        &self,
        user: &CurrentUser,
        dataframe_id: i32,
    ) -> Result<ModelDataFrameDownload, ModelDataFrameReadError> {
        let row = self
            .versions
            .find_version_dataframe(&user.id, dataframe_id)
            .await
            .map_err(map_repository)?;
        let file = self
            .reader
            .download(
                user,
                &version_source(&row),
                format!("model-v{}-{}", row.version_id, row.table_name),
            )
            .await?;
        Ok(ModelDataFrameDownload {
            path: file.path,
            file_name: file.file_name,
            media_type: file.media_type,
        })
    }

    pub async fn draft_metadata(
        &self,
        user: &CurrentUser,
        dataframe_id: i32,
        expectation: DraftReadExpectation,
    ) -> Result<ModelDataFrameMetadata, ModelDataFrameReadError> {
        let row = self.draft_row(user, dataframe_id, expectation).await?;
        draft_metadata(row)
    }

    pub async fn draft_page(
        &self,
        user: &CurrentUser,
        dataframe_id: i32,
        expectation: DraftReadExpectation,
        limit: Option<usize>,
        cursor: Option<&str>,
        columns: &[String],
    ) -> Result<ModelDataFramePage, ModelDataFrameReadError> {
        let row = self.draft_row(user, dataframe_id, expectation).await?;
        let page = self
            .reader
            .page(user, &draft_source(&row), limit, cursor, columns)
            .await?;
        Ok(ModelDataFramePage {
            resource_id: dataframe_id.to_string(),
            columns: page.columns,
            rows: page.rows,
            next_cursor: page.next_cursor,
            diagnostics: page.diagnostics,
        })
    }

    pub async fn draft_download(
        &self,
        user: &CurrentUser,
        dataframe_id: i32,
        expectation: DraftReadExpectation,
    ) -> Result<ModelDataFrameDownload, ModelDataFrameReadError> {
        let row = self.draft_row(user, dataframe_id, expectation).await?;
        let file = self
            .reader
            .download(
                user,
                &draft_source(&row),
                format!(
                    "draft-{}-r{}-{}",
                    row.draft_id, row.revision, row.table_name
                ),
            )
            .await?;
        Ok(ModelDataFrameDownload {
            path: file.path,
            file_name: file.file_name,
            media_type: file.media_type,
        })
    }

    async fn draft_row(
        &self,
        user: &CurrentUser,
        dataframe_id: i32,
        expectation: DraftReadExpectation,
    ) -> Result<DraftDataFrameReadRow, ModelDataFrameReadError> {
        self.versions
            .find_draft_dataframe(
                &user.id,
                dataframe_id,
                expectation.draft_id,
                expectation.expected_revision,
            )
            .await
            .map_err(map_repository)
    }
}

fn version_source(row: &VersionDataFrameReadRow) -> DataFrameReadSource {
    DataFrameReadSource {
        resource_key: format!("model-dataframe:{}", row.id),
        artifact_id: row.artifact_id,
        schema_json: row.schema_json.clone(),
        row_count: row.row_count,
        content_hash: row.content_hash.clone(),
    }
}

fn draft_source(row: &DraftDataFrameReadRow) -> DataFrameReadSource {
    DataFrameReadSource {
        resource_key: format!(
            "draft-dataframe:{}:draft:{}:revision:{}",
            row.id, row.draft_id, row.revision
        ),
        artifact_id: row.artifact_id,
        schema_json: row.schema_json.clone(),
        row_count: row.row_count,
        content_hash: row.content_hash.clone(),
    }
}

fn columns(schema_json: &str) -> Result<Vec<DataFrameColumn>, ModelDataFrameReadError> {
    serde_json::from_str(schema_json)
        .map_err(|error| ModelDataFrameReadError::ReadFailed(error.to_string()))
}

fn schema_hash(schema_json: &str) -> String {
    hex::encode(Sha256::digest(schema_json.as_bytes()))
}

fn version_metadata(
    row: VersionDataFrameReadRow,
) -> Result<ModelDataFrameMetadata, ModelDataFrameReadError> {
    Ok(ModelDataFrameMetadata {
        id: row.id.to_string(),
        scope: ModelDataFrameScope::ModelVersion,
        owner_id: row.version_id.to_string(),
        revision: None,
        table_name: row.table_name,
        row_count: row.row_count,
        size_bytes: row.size_bytes,
        columns: columns(&row.schema_json)?,
        schema_hash: schema_hash(&row.schema_json),
        content_hash: row.content_hash,
    })
}

fn draft_metadata(
    row: DraftDataFrameReadRow,
) -> Result<ModelDataFrameMetadata, ModelDataFrameReadError> {
    Ok(ModelDataFrameMetadata {
        id: row.id.to_string(),
        scope: ModelDataFrameScope::Draft,
        owner_id: row.draft_id.to_string(),
        revision: Some(row.revision),
        table_name: row.table_name,
        row_count: row.row_count,
        size_bytes: row.size_bytes,
        columns: columns(&row.schema_json)?,
        schema_hash: schema_hash(&row.schema_json),
        content_hash: row.content_hash,
    })
}
