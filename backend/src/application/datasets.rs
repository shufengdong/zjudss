use std::collections::BTreeMap;
use std::path::PathBuf;

use arrow_array::{
    Array, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array, Int64Array,
    LargeStringArray, StringArray, UInt8Array, UInt16Array, UInt32Array, UInt64Array,
};
use arrow_schema::DataType;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::application::ArtifactService;
use crate::auth::Principal;
use crate::db::models::{DatasetView, ResultDataset};
use crate::db::repositories::{DatasetRepo, RepoError};
use crate::storage::ParquetInspector;

#[derive(Debug)]
pub enum DatasetError {
    NotFound,
    InvalidCursor,
    InvalidPage(String),
    Storage(String),
    Repository(RepoError),
}
impl std::fmt::Display for DatasetError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "资源不存在"),
            Self::InvalidCursor => write!(f, "分页 cursor 无效"),
            Self::InvalidPage(v) => write!(f, "分页参数无效: {v}"),
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetColumn {
    pub name: String,
    pub dtype: String,
    pub nullable: bool,
    pub unit: Option<String>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetMetadata {
    pub id: String,
    pub task_id: Option<String>,
    pub analysis_run_id: Option<String>,
    pub name: String,
    pub row_count: i64,
    pub size_bytes: i64,
    pub columns: Vec<DatasetColumn>,
    pub views: Vec<DatasetViewDto>,
}
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetViewDto {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub spec: Value,
    pub is_default: bool,
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
    pub path: PathBuf,
    pub file_name: String,
    pub media_type: &'static str,
}

#[derive(Clone)]
pub struct DatasetService {
    repo: DatasetRepo,
    artifacts: ArtifactService,
}
impl DatasetService {
    pub fn new(repo: DatasetRepo, artifacts: ArtifactService) -> Self {
        Self { repo, artifacts }
    }
    pub async fn list(
        &self,
        p: &Principal,
        task_id: i32,
    ) -> Result<Vec<DatasetMetadata>, DatasetError> {
        let rows = self.repo.list_task(&p.access, task_id).await?;
        let mut out = Vec::new();
        for d in rows {
            out.push(self.metadata_from(p, d).await?);
        }
        Ok(out)
    }
    pub async fn metadata(&self, p: &Principal, id: i32) -> Result<DatasetMetadata, DatasetError> {
        let d = self.repo.find(&p.access, id).await?;
        self.metadata_from(p, d).await
    }
    async fn metadata_from(
        &self,
        p: &Principal,
        d: ResultDataset,
    ) -> Result<DatasetMetadata, DatasetError> {
        let columns: Vec<DatasetColumn> = serde_json::from_str(&d.schema_json)
            .map_err(|e| DatasetError::Storage(e.to_string()))?;
        let views = self
            .repo
            .views(&p.access, d.id)
            .await?
            .into_iter()
            .map(view_dto)
            .collect();
        Ok(DatasetMetadata {
            id: d.id.to_string(),
            task_id: d.task_id.map(|v| v.to_string()),
            analysis_run_id: d.analysis_run_id.map(|v| v.to_string()),
            name: d.result_name,
            row_count: d.row_count,
            size_bytes: d.size_bytes,
            columns,
            views,
        })
    }
    async fn path(&self, p: &Principal, d: &ResultDataset) -> Result<PathBuf, DatasetError> {
        let scope = self
            .artifacts
            .resolve_scope(&p.storage)
            .await
            .map_err(|e| DatasetError::Storage(e.to_string()))?;
        let a = self
            .artifacts
            .find_owned(scope.id, d.artifact_id)
            .await
            .map_err(|_| DatasetError::NotFound)?;
        self.artifacts
            .store()
            .resolve(&a.storage_path)
            .map_err(|e| DatasetError::Storage(e.to_string()))
    }
    pub async fn page(
        &self,
        p: &Principal,
        id: i32,
        limit: Option<usize>,
        cursor: Option<&str>,
        columns: &[String],
    ) -> Result<DatasetPage, DatasetError> {
        let d = self.repo.find(&p.access, id).await?;
        let limit = limit.unwrap_or(100);
        if !(1..=500).contains(&limit) {
            return Err(DatasetError::InvalidPage("limit 必须在 1..=500".into()));
        }
        let offset = match cursor {
            Some(c) => decode_cursor(c, id, &d.schema_hash)?,
            None => 0,
        };
        let path = self.path(p, &d).await?;
        let batches = ParquetInspector::projection(&path, columns, offset, limit)
            .map_err(|e| DatasetError::Storage(e.to_string()))?;
        let names = if let Some(batch) = batches.first() {
            batch
                .schema()
                .fields()
                .iter()
                .map(|f| f.name().clone())
                .collect()
        } else if columns.is_empty() {
            serde_json::from_str::<Vec<DatasetColumn>>(&d.schema_json)
                .map_err(|e| DatasetError::Storage(e.to_string()))?
                .into_iter()
                .map(|c| c.name)
                .collect()
        } else {
            columns.to_vec()
        };
        let mut rows = Vec::new();
        let mut diagnostics = BTreeMap::new();
        for batch in batches {
            for row in 0..batch.num_rows() {
                let mut values = Vec::with_capacity(batch.num_columns());
                for array in batch.columns() {
                    values.push(value(array.as_ref(), row, &mut diagnostics)?);
                }
                rows.push(values);
            }
        }
        let consumed = rows.len();
        let next = if (offset + consumed) < usize::try_from(d.row_count).unwrap_or(usize::MAX) {
            Some(encode_cursor(id, &d.schema_hash, offset + consumed))
        } else {
            None
        };
        Ok(DatasetPage {
            dataset_id: id.to_string(),
            columns: names,
            rows,
            next_cursor: next,
            diagnostics,
        })
    }
    pub async fn download(&self, p: &Principal, id: i32) -> Result<DatasetDownload, DatasetError> {
        let d = self.repo.find(&p.access, id).await?;
        let path = self.path(p, &d).await?;
        let safe = d
            .result_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>();
        Ok(DatasetDownload {
            path,
            file_name: format!("{}-{}.parquet", d.id, safe),
            media_type: "application/vnd.apache.parquet",
        })
    }
}
fn view_dto(v: DatasetView) -> DatasetViewDto {
    DatasetViewDto {
        id: v.id.to_string(),
        name: v.name,
        kind: v.kind,
        spec: serde_json::from_str(&v.spec_json).unwrap_or(Value::Null),
        is_default: v.is_default,
    }
}
fn encode_cursor(id: i32, hash: &str, offset: usize) -> String {
    hex(format!("v1|{id}|{hash}|0|{offset}").as_bytes())
}
fn decode_cursor(s: &str, id: i32, hash: &str) -> Result<usize, DatasetError> {
    let bytes = unhex(s).ok_or(DatasetError::InvalidCursor)?;
    let text = String::from_utf8(bytes).map_err(|_| DatasetError::InvalidCursor)?;
    let p = text.split('|').collect::<Vec<_>>();
    if p.len() != 5 || p[0] != "v1" || p[1] != id.to_string() || p[2] != hash || p[3] != "0" {
        return Err(DatasetError::InvalidCursor);
    }
    p[4].parse().map_err(|_| DatasetError::InvalidCursor)
}
fn hex(v: &[u8]) -> String {
    v.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(v: &str) -> Option<Vec<u8>> {
    if !v.len().is_multiple_of(2) {
        return None;
    }
    (0..v.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&v[i..i + 2], 16).ok())
        .collect()
}
pub fn schema_json_and_hash(meta: &crate::storage::ParquetMetadata) -> (String, String) {
    let cols = meta
        .columns
        .iter()
        .map(|c| DatasetColumn {
            name: c.name.clone(),
            dtype: c.dtype.clone(),
            nullable: c.nullable,
            unit: None,
        })
        .collect::<Vec<_>>();
    let json = serde_json::to_string(&cols).expect("schema serializable");
    let hash = format!("{:x}", Sha256::digest(json.as_bytes()));
    (json, hash)
}
fn null(diag: &mut BTreeMap<String, u64>, kind: &str) -> Value {
    *diag.entry(kind.into()).or_default() += 1;
    Value::Null
}
fn value(a: &dyn Array, row: usize, d: &mut BTreeMap<String, u64>) -> Result<Value, DatasetError> {
    if a.is_null(row) {
        return Ok(null(d, "missingValues"));
    }
    macro_rules! n {
        ($t:ty) => {
            if let Some(x) = a.as_any().downcast_ref::<$t>() {
                return Ok(Value::from(x.value(row)));
            }
        };
    }
    match a.data_type() {
        DataType::Utf8 => Ok(Value::String(
            a.as_any()
                .downcast_ref::<StringArray>()
                .unwrap()
                .value(row)
                .into(),
        )),
        DataType::LargeUtf8 => Ok(Value::String(
            a.as_any()
                .downcast_ref::<LargeStringArray>()
                .unwrap()
                .value(row)
                .into(),
        )),
        DataType::Boolean => Ok(Value::Bool(
            a.as_any()
                .downcast_ref::<BooleanArray>()
                .unwrap()
                .value(row),
        )),
        DataType::Int8 => {
            n!(Int8Array);
            unreachable!()
        }
        DataType::Int16 => {
            n!(Int16Array);
            unreachable!()
        }
        DataType::Int32 => {
            n!(Int32Array);
            unreachable!()
        }
        DataType::Int64 => {
            n!(Int64Array);
            unreachable!()
        }
        DataType::UInt8 => {
            n!(UInt8Array);
            unreachable!()
        }
        DataType::UInt16 => {
            n!(UInt16Array);
            unreachable!()
        }
        DataType::UInt32 => {
            n!(UInt32Array);
            unreachable!()
        }
        DataType::UInt64 => {
            let x = a.as_any().downcast_ref::<UInt64Array>().unwrap().value(row);
            Ok(serde_json::Number::from_u128(x as u128)
                .map(Value::Number)
                .unwrap_or_else(|| null(d, "outOfRangeValues")))
        }
        DataType::Float32 => {
            let x = a
                .as_any()
                .downcast_ref::<Float32Array>()
                .unwrap()
                .value(row) as f64;
            if x.is_finite() {
                Ok(Value::from(x))
            } else {
                Ok(null(d, "nonFiniteValues"))
            }
        }
        DataType::Float64 => {
            let x = a
                .as_any()
                .downcast_ref::<Float64Array>()
                .unwrap()
                .value(row);
            if x.is_finite() {
                Ok(Value::from(x))
            } else {
                Ok(null(d, "nonFiniteValues"))
            }
        }
        other => Err(DatasetError::Storage(format!("不支持的数据类型 {other}"))),
    }
}
