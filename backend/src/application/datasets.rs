use std::collections::BTreeMap;
use std::fs::File;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::application::ArtifactService;
use crate::application::dataframe_reader::{DataFrameReadError, ParquetDataFrameReader};
use crate::application::result_sources::{ResultSourceError, ResultSourceService};
use crate::auth::CurrentUser;
use crate::db::repositories::{
    DeleteResultError, LineageRow, RepoError, ResultRepo, ResultViewRow, RunDataset,
};

pub use crate::application::dataframe_reader::DataFrameColumn as DatasetColumn;

#[derive(Debug)]
pub enum DatasetError {
    NotFound,
    InvalidCursor,
    InvalidPage(String),
    Busy,
    Storage(String),
    Repository(RepoError),
}
impl std::fmt::Display for DatasetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "资源不存在"),
            Self::InvalidCursor => write!(f, "分页 cursor 无效"),
            Self::InvalidPage(v) => write!(f, "分页参数无效: {v}"),
            Self::Busy => write!(f, "结果正被运行中的分析使用"),
            Self::Storage(v) => write!(f, "数据集读取失败: {v}"),
            Self::Repository(v) => write!(f, "{v}"),
        }
    }
}
impl std::error::Error for DatasetError {}
impl From<RepoError> for DatasetError {
    fn from(v: RepoError) -> Self {
        if matches!(v, RepoError::NotFound) {
            Self::NotFound
        } else {
            Self::Repository(v)
        }
    }
}
impl From<DataFrameReadError> for DatasetError {
    fn from(value: DataFrameReadError) -> Self {
        match value {
            DataFrameReadError::NotFound => Self::NotFound,
            DataFrameReadError::InvalidCursor => Self::InvalidCursor,
            DataFrameReadError::InvalidPage(message) => Self::InvalidPage(message),
            DataFrameReadError::TooManyRows(limit) => {
                Self::Storage(format!("DataFrame 扫描行数超过限制 {limit}"))
            }
            DataFrameReadError::Storage(message) => Self::Storage(message),
        }
    }
}
impl From<ResultSourceError> for DatasetError {
    fn from(value: ResultSourceError) -> Self {
        match value {
            ResultSourceError::NotFound => Self::NotFound,
            ResultSourceError::Repository(error) => error.into(),
            ResultSourceError::Artifact(crate::application::ArtifactError::NotFound) => {
                Self::NotFound
            }
            ResultSourceError::Artifact(error) => Self::Storage(error.to_string()),
            ResultSourceError::Stored(message) => Self::Storage(message),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetMetadata {
    pub id: String,
    pub run_id: Option<String>,
    pub analysis_run_id: Option<String>,
    pub name: String,
    pub row_count: i64,
    pub size_bytes: i64,
    pub columns: Vec<DatasetColumn>,
    pub views: Vec<DatasetViewDto>,
    pub schema_hash: String,
    pub lineage: Vec<DatasetLineageDto>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetViewDto {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub created_at: i64,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetLineageDto {
    pub source_dataset_id: Option<String>,
    pub source_resource_id: Option<String>,
    pub name: String,
    pub schema: Value,
    pub schema_hash: String,
    pub content_hash: String,
    pub producer: Value,
    pub deleted: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tombstone: Option<Value>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetPage {
    pub dataset_id: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub next_cursor: Option<String>,
    pub diagnostics: BTreeMap<String, u64>,
}
pub struct DatasetDownload {
    pub file: File,
    pub file_name: String,
    pub media_type: &'static str,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedDatasetDto {
    pub dataset_id: String,
    pub tombstone: Value,
}

#[derive(Clone)]
pub struct DatasetService {
    repo: ResultRepo,
    artifacts: ArtifactService,
    reader: ParquetDataFrameReader,
    sources: ResultSourceService,
    messages: crate::db::repositories::MessageRepo,
    events: crate::application::EventBus,
}
impl DatasetService {
    pub fn new(
        repo: ResultRepo,
        artifacts: ArtifactService,
        messages: crate::db::repositories::MessageRepo,
        events: crate::application::EventBus,
    ) -> Self {
        let reader = ParquetDataFrameReader::new(artifacts.clone());
        let sources = ResultSourceService::new(repo.clone(), artifacts.clone());
        Self {
            repo,
            artifacts,
            reader,
            sources,
            messages,
            events,
        }
    }
    pub async fn list(
        &self,
        p: &CurrentUser,
        run_id: i32,
    ) -> Result<Vec<DatasetMetadata>, DatasetError> {
        let rows = self.repo.list_run(&p.id, run_id).await?;
        let mut out = Vec::new();
        for d in rows {
            out.push(self.metadata_from(p, d).await?);
        }
        Ok(out)
    }
    pub async fn list_analysis(
        &self,
        p: &CurrentUser,
        id: i32,
    ) -> Result<Vec<DatasetMetadata>, DatasetError> {
        let rows = self.repo.list_analysis(&p.id, id).await?;
        let mut out = Vec::new();
        for d in rows {
            out.push(self.metadata_from(p, d).await?);
        }
        Ok(out)
    }
    pub async fn metadata(
        &self,
        p: &CurrentUser,
        id: i32,
    ) -> Result<DatasetMetadata, DatasetError> {
        let d = self.repo.find_live(&p.id, id).await?;
        self.metadata_from(p, d).await
    }
    async fn metadata_from(
        &self,
        p: &CurrentUser,
        d: RunDataset,
    ) -> Result<DatasetMetadata, DatasetError> {
        let columns: Vec<DatasetColumn> = serde_json::from_str(&d.schema_json)
            .map_err(|e| DatasetError::Storage(e.to_string()))?;
        let views = self
            .repo
            .views(&p.id, d.id)
            .await?
            .into_iter()
            .map(view_dto)
            .collect();
        let lineage = self
            .repo
            .lineage(&p.id, d.id)
            .await?
            .into_iter()
            .map(lineage_dto)
            .collect();
        Ok(DatasetMetadata {
            id: d.id.to_string(),
            run_id: d.run_id.map(|v| v.to_string()),
            analysis_run_id: d.analysis_run_id.map(|v| v.to_string()),
            name: d.result_name,
            row_count: d.row_count,
            size_bytes: d.size_bytes,
            columns,
            views,
            schema_hash: d.schema_hash,
            lineage,
        })
    }
    pub async fn page(
        &self,
        p: &CurrentUser,
        id: i32,
        limit: Option<usize>,
        cursor: Option<&str>,
        columns: &[String],
    ) -> Result<DatasetPage, DatasetError> {
        let source = self.sources.resolve(p, id).await?;
        let page = self
            .reader
            .page(p, &source.read_source, limit, cursor, columns)
            .await?;
        Ok(DatasetPage {
            dataset_id: id.to_string(),
            columns: page.columns,
            rows: page.rows,
            next_cursor: page.next_cursor,
            diagnostics: page.diagnostics,
        })
    }
    pub async fn download(
        &self,
        p: &CurrentUser,
        id: i32,
    ) -> Result<DatasetDownload, DatasetError> {
        let source = self.sources.resolve(p, id).await?;
        let file = self
            .reader
            .download(
                p,
                &source.read_source,
                format!("{}-{}", source.dataset.id, source.dataset.result_name),
            )
            .await?;
        Ok(DatasetDownload {
            file: file.file,
            file_name: file.file_name,
            media_type: file.media_type,
        })
    }
    pub async fn delete(
        &self,
        p: &CurrentUser,
        id: i32,
    ) -> Result<DeletedDatasetDto, DatasetError> {
        let deleted = self
            .repo
            .delete(&p.id, id, crate::application::now_ms())
            .await
            .map_err(|error| match error {
                DeleteResultError::Busy => DatasetError::Busy,
                DeleteResultError::Repository(RepoError::NotFound) => DatasetError::NotFound,
                DeleteResultError::Repository(error) => DatasetError::Repository(error),
            })?;
        self.artifacts
            .release(p.storage_scope_id, deleted.artifact_id)
            .await
            .map_err(|error| DatasetError::Storage(error.to_string()))?;
        self.artifacts
            .collect_unreferenced()
            .await
            .map_err(|error| DatasetError::Storage(error.to_string()))?;
        let tombstone: Value = serde_json::from_str(&deleted.tombstone_json)
            .map_err(|error| DatasetError::Storage(error.to_string()))?;
        let writer = crate::application::EventWriter::new(
            p.id.clone(),
            deleted.session_id,
            self.messages.clone(),
            self.events.clone(),
        );
        writer
            .assistant(&format!("结果数据集 {id} 已删除"))
            .await
            .map_err(DatasetError::Repository)?;
        Ok(DeletedDatasetDto {
            dataset_id: deleted.dataset_id.to_string(),
            tombstone,
        })
    }
}
fn view_dto(v: ResultViewRow) -> DatasetViewDto {
    DatasetViewDto {
        id: v.id.to_string(),
        name: v.name,
        kind: v.kind,
        created_at: v.created_at_ms,
    }
}
fn lineage_dto(value: LineageRow) -> DatasetLineageDto {
    DatasetLineageDto {
        source_dataset_id: value.source_dataset_id.map(|id| id.to_string()),
        source_resource_id: value.source_resource_id,
        name: value.source_name,
        schema: serde_json::from_str(&value.source_schema_json).unwrap_or(Value::Null),
        schema_hash: value.source_schema_hash,
        content_hash: value.source_content_hash,
        producer: serde_json::from_str(&value.source_producer_json).unwrap_or(Value::Null),
        deleted: value.deleted,
        tombstone: value
            .tombstone_json
            .and_then(|json| serde_json::from_str(&json).ok()),
    }
}
pub fn schema_json_and_hash(meta: &crate::storage::ParquetMetadata) -> (String, String) {
    let cols = meta
        .columns
        .iter()
        .map(|c| DatasetColumn {
            name: c.name.clone(),
            dtype: c.dtype.clone(),
            nullable: c.nullable,
            unit: c.unit.clone(),
        })
        .collect::<Vec<_>>();
    let json = serde_json::to_string(&cols).expect("schema serializable");
    let hash = hex::encode(Sha256::digest(json.as_bytes()));
    (json, hash)
}
