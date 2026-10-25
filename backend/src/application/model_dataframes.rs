use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;

use arrow_array::{
    Array, ArrayRef, BooleanArray, Float32Array, Float64Array, Int8Array, Int16Array, Int32Array,
    Int64Array, LargeStringArray, StringArray, StringViewArray, UInt8Array, UInt16Array,
    UInt32Array, UInt64Array,
};
use arrow_schema::DataType;
use parquet::arrow::{ProjectionMask, arrow_reader::ParquetRecordBatchReaderBuilder};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::application::dataframe_reader::{
    DataFrameColumn, DataFrameReadError, DataFrameReadSource, ParquetDataFrameReader, value,
};
use crate::auth::CurrentUser;
use crate::db::models::DraftRef;
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

#[derive(Debug, Clone, Copy, serde::Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelReadScope {
    Active,
    Draft,
}

#[derive(Debug, Clone)]
pub struct ModelDataFrameQuery {
    pub scope: ModelReadScope,
    pub table_name: String,
    pub columns: Vec<String>,
    pub equals: BTreeMap<String, Value>,
    pub limit: usize,
    pub expected_draft: Option<DraftRef>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelDataFrameQueryResult {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub match_count: u64,
    pub truncated: bool,
    pub actual_scope: ModelReadScope,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub draft_id: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub revision: Option<i64>,
    pub content_hash: String,
}

pub struct ModelDataFrameDownload {
    pub file: File,
    pub file_name: String,
    pub media_type: &'static str,
}

#[derive(Debug)]
pub enum ModelDataFrameReadError {
    NotFound,
    DraftRevisionConflict,
    ModelScopeUnavailable,
    TableNotFound,
    InvalidModelQuery(String),
    ModelQueryOutputTooLarge,
    InvalidPage(String),
    ReadFailed(String),
    Repository(RepoError),
}

impl std::fmt::Display for ModelDataFrameReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "资源不存在"),
            Self::DraftRevisionConflict => write!(formatter, "草稿已变化，请刷新后重试"),
            Self::ModelScopeUnavailable => write!(formatter, "请求的模型范围当前不可用"),
            Self::TableNotFound => write!(formatter, "当前模型范围中不存在该表"),
            Self::InvalidModelQuery(message) => write!(formatter, "模型查询无效: {message}"),
            Self::ModelQueryOutputTooLarge => {
                write!(formatter, "模型查询输出超过 64 KiB，请缩小列投影或查询范围")
            }
            Self::InvalidPage(message) => write!(formatter, "分页参数无效: {message}"),
            Self::ReadFailed(message) => write!(formatter, "DataFrame 读取失败: {message}"),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ModelDataFrameReadError {}

impl ModelDataFrameReadError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::DraftRevisionConflict => "draft_revision_conflict",
            Self::ModelScopeUnavailable => "model_scope_unavailable",
            Self::TableNotFound => "table_not_found",
            Self::InvalidModelQuery(_) => "invalid_model_query",
            Self::ModelQueryOutputTooLarge => "model_query_output_too_large",
            Self::InvalidPage(_) => "invalid_page",
            Self::ReadFailed(_) | Self::Repository(_) => "dataframe_read_failed",
        }
    }
}

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
            resource_id: format!("model-dataframe:{dataframe_id}"),
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
            file: file.file,
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
            resource_id: format!(
                "draft-dataframe:{dataframe_id}:revision:{}",
                expectation.expected_revision
            ),
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
            file: file.file,
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

    pub async fn query_workspace(
        &self,
        user: &CurrentUser,
        session_id: i32,
        query: ModelDataFrameQuery,
    ) -> Result<ModelDataFrameQueryResult, ModelDataFrameReadError> {
        validate_query_shape(&query)?;
        let workspace = self
            .versions
            .workspace(&user.id, session_id, crate::application::now_ms())
            .await
            .map_err(map_repository)?;
        let (source, version_id, draft_id, revision) = match query.scope {
            ModelReadScope::Active => {
                if query.expected_draft.is_some() {
                    return Err(ModelDataFrameReadError::InvalidModelQuery(
                        "active 查询不能提供 expectedDraft".into(),
                    ));
                }
                let version = workspace
                    .active
                    .ok_or(ModelDataFrameReadError::ModelScopeUnavailable)?;
                let frame = version
                    .dataframes
                    .iter()
                    .find(|frame| frame.table_name == query.table_name)
                    .ok_or(ModelDataFrameReadError::TableNotFound)?;
                (
                    DataFrameReadSource {
                        resource_key: format!("model-dataframe:{}", frame.id),
                        artifact_id: frame.artifact_id,
                        schema_json: frame.schema_json.clone(),
                        row_count: frame.row_count,
                        content_hash: frame.content_hash.clone(),
                    },
                    Some(version.version.id),
                    None,
                    None,
                )
            }
            ModelReadScope::Draft => {
                let expected = query.expected_draft.ok_or_else(|| {
                    ModelDataFrameReadError::InvalidModelQuery(
                        "draft 查询必须提供 expectedDraft".into(),
                    )
                })?;
                let draft = workspace
                    .draft
                    .ok_or(ModelDataFrameReadError::ModelScopeUnavailable)?;
                if draft.draft.id != expected.id || draft.draft.revision != expected.revision {
                    return Err(ModelDataFrameReadError::DraftRevisionConflict);
                }
                let frame = draft
                    .dataframes
                    .iter()
                    .find(|frame| frame.table_name == query.table_name)
                    .ok_or(ModelDataFrameReadError::TableNotFound)?;
                (
                    DataFrameReadSource {
                        resource_key: format!(
                            "draft-dataframe:{}:draft:{}:revision:{}",
                            frame.id, draft.draft.id, draft.draft.revision
                        ),
                        artifact_id: frame.artifact_id,
                        schema_json: frame.schema_json.clone(),
                        row_count: frame.row_count,
                        content_hash: frame.content_hash.clone(),
                    },
                    None,
                    Some(draft.draft.id),
                    Some(draft.draft.revision),
                )
            }
        };
        validate_registered_columns(&source.schema_json, &query.columns, &query.equals)?;
        let file = self.reader.open_owned(user, source.artifact_id).await?;
        let (rows, match_count) = scan_query(file, &query)?;
        let result = ModelDataFrameQueryResult {
            columns: query.columns,
            truncated: match_count > rows.len() as u64,
            rows,
            match_count,
            actual_scope: query.scope,
            version_id,
            draft_id,
            revision,
            content_hash: source.content_hash,
        };
        if serde_json::to_vec(&result)
            .map_err(|error| ModelDataFrameReadError::ReadFailed(error.to_string()))?
            .len()
            > 64 * 1024
        {
            return Err(ModelDataFrameReadError::ModelQueryOutputTooLarge);
        }
        Ok(result)
    }
}

fn validate_query_shape(query: &ModelDataFrameQuery) -> Result<(), ModelDataFrameReadError> {
    if query.table_name.trim().is_empty() {
        return Err(ModelDataFrameReadError::InvalidModelQuery(
            "tableName 不能为空".into(),
        ));
    }
    if !(1..=100).contains(&query.limit) {
        return Err(ModelDataFrameReadError::InvalidModelQuery(
            "limit 必须在 1..=100".into(),
        ));
    }
    if query.columns.len() > 32 {
        return Err(ModelDataFrameReadError::InvalidModelQuery(
            "columns 最多 32 列".into(),
        ));
    }
    if query.equals.len() > 8 {
        return Err(ModelDataFrameReadError::InvalidModelQuery(
            "equals 最多 8 列".into(),
        ));
    }
    if query.columns.iter().collect::<BTreeSet<_>>().len() != query.columns.len() {
        return Err(ModelDataFrameReadError::InvalidModelQuery(
            "columns 不能重复".into(),
        ));
    }
    Ok(())
}

fn validate_registered_columns(
    schema_json: &str,
    output: &[String],
    equals: &BTreeMap<String, Value>,
) -> Result<(), ModelDataFrameReadError> {
    let available = columns(schema_json)?
        .into_iter()
        .map(|column| column.name)
        .collect::<BTreeSet<_>>();
    for name in output.iter().chain(equals.keys()) {
        if !available.contains(name) {
            return Err(ModelDataFrameReadError::InvalidModelQuery(format!(
                "列不存在: {name}"
            )));
        }
    }
    Ok(())
}

#[derive(Debug)]
enum TypedPredicate {
    String(String),
    Boolean(bool),
    Signed(i64),
    Unsigned(u64),
    Float32(f32),
    Float64(f64),
}

fn scan_query(
    file: File,
    query: &ModelDataFrameQuery,
) -> Result<(Vec<Vec<Value>>, u64), ModelDataFrameReadError> {
    let mut builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|error| ModelDataFrameReadError::ReadFailed(error.to_string()))?;
    let schema = builder.schema().clone();
    let predicates = query
        .equals
        .iter()
        .map(|(name, expected)| {
            let field = schema.field_with_name(name).map_err(|_| {
                ModelDataFrameReadError::InvalidModelQuery(format!("列不存在: {name}"))
            })?;
            Ok((name.clone(), typed_predicate(field.data_type(), expected)?))
        })
        .collect::<Result<Vec<_>, ModelDataFrameReadError>>()?;
    let projection = query
        .columns
        .iter()
        .chain(query.equals.keys())
        .cloned()
        .collect::<BTreeSet<_>>();
    if !projection.is_empty() {
        let mask = ProjectionMask::columns(
            builder.parquet_schema(),
            projection.iter().map(String::as_str),
        );
        builder = builder.with_projection(mask);
    }
    let reader = builder
        .build()
        .map_err(|error| ModelDataFrameReadError::ReadFailed(error.to_string()))?;
    let mut rows = Vec::new();
    let mut match_count = 0_u64;
    let mut diagnostics = BTreeMap::new();
    for batch in reader {
        let batch =
            batch.map_err(|error| ModelDataFrameReadError::ReadFailed(error.to_string()))?;
        let batch_schema = batch.schema();
        for row in 0..batch.num_rows() {
            let matched = predicates.iter().all(|(name, predicate)| {
                batch_schema
                    .index_of(name)
                    .ok()
                    .is_some_and(|index| predicate_matches(batch.column(index), row, predicate))
            });
            if !matched {
                continue;
            }
            match_count = match_count.saturating_add(1);
            if rows.len() >= query.limit {
                continue;
            }
            let projected = query
                .columns
                .iter()
                .map(|name| {
                    let index = batch_schema.index_of(name).map_err(|_| {
                        ModelDataFrameReadError::InvalidModelQuery(format!("列不存在: {name}"))
                    })?;
                    value(batch.column(index).as_ref(), row, &mut diagnostics).map_err(Into::into)
                })
                .collect::<Result<Vec<_>, ModelDataFrameReadError>>()?;
            if serde_json::to_vec(&projected)
                .map_err(|error| ModelDataFrameReadError::ReadFailed(error.to_string()))?
                .len()
                > 64 * 1024
            {
                return Err(ModelDataFrameReadError::ModelQueryOutputTooLarge);
            }
            rows.push(projected);
        }
    }
    Ok((rows, match_count))
}

fn typed_predicate(
    data_type: &DataType,
    expected: &Value,
) -> Result<TypedPredicate, ModelDataFrameReadError> {
    let invalid = || {
        ModelDataFrameReadError::InvalidModelQuery(format!(
            "值 {expected} 与列类型 {data_type} 不兼容"
        ))
    };
    match data_type {
        DataType::Utf8 | DataType::LargeUtf8 | DataType::Utf8View => expected
            .as_str()
            .map(|value| TypedPredicate::String(value.to_string()))
            .ok_or_else(invalid),
        DataType::Boolean => expected
            .as_bool()
            .map(TypedPredicate::Boolean)
            .ok_or_else(invalid),
        DataType::Int8 => signed(expected, i8::MIN as i64, i8::MAX as i64),
        DataType::Int16 => signed(expected, i16::MIN as i64, i16::MAX as i64),
        DataType::Int32 => signed(expected, i32::MIN as i64, i32::MAX as i64),
        DataType::Int64 => signed(expected, i64::MIN, i64::MAX),
        DataType::UInt8 => unsigned(expected, u8::MAX as u64),
        DataType::UInt16 => unsigned(expected, u16::MAX as u64),
        DataType::UInt32 => unsigned(expected, u32::MAX as u64),
        DataType::UInt64 => unsigned(expected, u64::MAX),
        DataType::Float32 => expected
            .as_f64()
            .filter(|_| expected.as_i64().is_none() && expected.as_u64().is_none())
            .filter(|value| value.is_finite())
            .map(|value| value as f32)
            .filter(|value| value.is_finite())
            .map(TypedPredicate::Float32)
            .ok_or_else(invalid),
        DataType::Float64 => expected
            .as_f64()
            .filter(|_| expected.as_i64().is_none() && expected.as_u64().is_none())
            .filter(|value| value.is_finite())
            .map(TypedPredicate::Float64)
            .ok_or_else(invalid),
        _ => Err(invalid()),
    }
}

fn signed(
    expected: &Value,
    minimum: i64,
    maximum: i64,
) -> Result<TypedPredicate, ModelDataFrameReadError> {
    let value = expected
        .as_i64()
        .or_else(|| {
            expected
                .as_u64()
                .and_then(|value| i64::try_from(value).ok())
        })
        .filter(|value| (*value >= minimum) && (*value <= maximum))
        .ok_or_else(|| {
            ModelDataFrameReadError::InvalidModelQuery("整数必须无损落入目标有符号类型".into())
        })?;
    Ok(TypedPredicate::Signed(value))
}

fn unsigned(expected: &Value, maximum: u64) -> Result<TypedPredicate, ModelDataFrameReadError> {
    let value = expected
        .as_u64()
        .or_else(|| {
            expected
                .as_i64()
                .and_then(|value| u64::try_from(value).ok())
        })
        .filter(|value| *value <= maximum)
        .ok_or_else(|| {
            ModelDataFrameReadError::InvalidModelQuery("整数必须无损落入目标无符号类型".into())
        })?;
    Ok(TypedPredicate::Unsigned(value))
}

fn predicate_matches(array: &ArrayRef, row: usize, predicate: &TypedPredicate) -> bool {
    if array.is_null(row) {
        return false;
    }
    match predicate {
        TypedPredicate::String(expected) => match array.data_type() {
            DataType::Utf8 => array
                .as_any()
                .downcast_ref::<StringArray>()
                .is_some_and(|array| array.value(row) == expected),
            DataType::LargeUtf8 => array
                .as_any()
                .downcast_ref::<LargeStringArray>()
                .is_some_and(|array| array.value(row) == expected),
            DataType::Utf8View => array
                .as_any()
                .downcast_ref::<StringViewArray>()
                .is_some_and(|array| array.value(row) == expected),
            _ => false,
        },
        TypedPredicate::Boolean(expected) => array
            .as_any()
            .downcast_ref::<BooleanArray>()
            .is_some_and(|array| array.value(row) == *expected),
        TypedPredicate::Signed(expected) => signed_at(array, row) == Some(*expected),
        TypedPredicate::Unsigned(expected) => unsigned_at(array, row) == Some(*expected),
        TypedPredicate::Float32(expected) => array
            .as_any()
            .downcast_ref::<Float32Array>()
            .is_some_and(|array| array.value(row) == *expected),
        TypedPredicate::Float64(expected) => array
            .as_any()
            .downcast_ref::<Float64Array>()
            .is_some_and(|array| array.value(row) == *expected),
    }
}

fn signed_at(array: &ArrayRef, row: usize) -> Option<i64> {
    macro_rules! get {
        ($ty:ty) => {
            array
                .as_any()
                .downcast_ref::<$ty>()
                .map(|array| i64::from(array.value(row)))
        };
    }
    match array.data_type() {
        DataType::Int8 => get!(Int8Array),
        DataType::Int16 => get!(Int16Array),
        DataType::Int32 => get!(Int32Array),
        DataType::Int64 => array
            .as_any()
            .downcast_ref::<Int64Array>()
            .map(|array| array.value(row)),
        _ => None,
    }
}

fn unsigned_at(array: &ArrayRef, row: usize) -> Option<u64> {
    macro_rules! get {
        ($ty:ty) => {
            array
                .as_any()
                .downcast_ref::<$ty>()
                .map(|array| u64::from(array.value(row)))
        };
    }
    match array.data_type() {
        DataType::UInt8 => get!(UInt8Array),
        DataType::UInt16 => get!(UInt16Array),
        DataType::UInt32 => get!(UInt32Array),
        DataType::UInt64 => array
            .as_any()
            .downcast_ref::<UInt64Array>()
            .map(|array| array.value(row)),
        _ => None,
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

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use arrow_array::{
        BooleanArray, Float32Array, Float64Array, Int8Array, Int64Array, LargeStringArray,
        StringArray, StringViewArray, UInt8Array, UInt64Array,
    };
    use serde_json::json;

    use super::*;

    #[test]
    fn typed_predicates_match_supported_arrow_scalars_exactly() {
        let cases: Vec<(ArrayRef, DataType, Value)> = vec![
            (
                Arc::new(StringArray::from(vec!["s"])),
                DataType::Utf8,
                json!("s"),
            ),
            (
                Arc::new(LargeStringArray::from(vec!["large"])),
                DataType::LargeUtf8,
                json!("large"),
            ),
            (
                Arc::new(StringViewArray::from(vec!["view"])),
                DataType::Utf8View,
                json!("view"),
            ),
            (
                Arc::new(BooleanArray::from(vec![true])),
                DataType::Boolean,
                json!(true),
            ),
            (
                Arc::new(Int8Array::from(vec![-8])),
                DataType::Int8,
                json!(-8),
            ),
            (
                Arc::new(Int64Array::from(vec![i64::MAX])),
                DataType::Int64,
                json!(i64::MAX),
            ),
            (
                Arc::new(UInt8Array::from(vec![8])),
                DataType::UInt8,
                json!(8),
            ),
            (
                Arc::new(UInt64Array::from(vec![u64::MAX])),
                DataType::UInt64,
                json!(u64::MAX),
            ),
            (
                Arc::new(Float32Array::from(vec![1.5])),
                DataType::Float32,
                json!(1.5),
            ),
            (
                Arc::new(Float64Array::from(vec![2.5])),
                DataType::Float64,
                json!(2.5),
            ),
        ];
        for (array, data_type, expected) in cases {
            let predicate = typed_predicate(&data_type, &expected).unwrap();
            assert!(predicate_matches(&array, 0, &predicate), "{data_type}");
        }
    }

    #[test]
    fn typed_predicates_reject_lossy_or_complex_values() {
        assert!(typed_predicate(&DataType::Int8, &json!(128)).is_err());
        assert!(typed_predicate(&DataType::UInt8, &json!(-1)).is_err());
        assert!(typed_predicate(&DataType::Int64, &json!(1.0)).is_err());
        assert!(typed_predicate(&DataType::Float64, &json!(1)).is_err());
        for value in [Value::Null, json!([1]), json!({"value": 1})] {
            assert!(typed_predicate(&DataType::Utf8, &value).is_err());
        }
        let list_type = DataType::List(Arc::new(arrow_schema::Field::new(
            "item",
            DataType::Int64,
            true,
        )));
        assert!(typed_predicate(&list_type, &json!(1)).is_err());
    }

    #[test]
    fn query_shape_enforces_column_filter_and_limit_bounds() {
        let query = |columns: usize, filters: usize, limit: usize| ModelDataFrameQuery {
            scope: ModelReadScope::Active,
            table_name: "bus".into(),
            columns: (0..columns).map(|index| format!("c{index}")).collect(),
            equals: (0..filters)
                .map(|index| (format!("f{index}"), json!("v")))
                .collect(),
            limit,
            expected_draft: None,
        };
        assert!(validate_query_shape(&query(32, 8, 100)).is_ok());
        assert!(validate_query_shape(&query(33, 0, 1)).is_err());
        assert!(validate_query_shape(&query(1, 9, 1)).is_err());
        assert!(validate_query_shape(&query(1, 1, 0)).is_err());
        assert!(validate_query_shape(&query(1, 1, 101)).is_err());
    }
}
