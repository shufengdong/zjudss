use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;

use arrow_array::{
    Array, ArrayRef, BooleanArray, FixedSizeListArray, Float32Array, Float64Array, Int8Array,
    Int16Array, Int32Array, Int64Array, LargeListArray, LargeListViewArray, LargeStringArray,
    ListArray, ListViewArray, RecordBatch, StringArray, StringViewArray, UInt8Array, UInt16Array,
    UInt32Array, UInt64Array,
};
use arrow_schema::DataType;
use parquet::arrow::{ProjectionMask, arrow_reader::ParquetRecordBatchReaderBuilder};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::application::ArtifactService;
use crate::auth::CurrentUser;
use crate::storage::ParquetInspector;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFrameColumn {
    pub name: String,
    pub dtype: String,
    pub nullable: bool,
    pub unit: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct DataFrameReadSource {
    pub resource_key: String,
    pub artifact_id: i32,
    pub schema_json: String,
    pub row_count: i64,
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub(crate) struct DataFramePageCore {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub next_cursor: Option<String>,
    pub diagnostics: BTreeMap<String, u64>,
}

pub(crate) struct DataFrameDownload {
    pub file: File,
    pub file_name: String,
    pub media_type: &'static str,
}

#[derive(Debug)]
pub(crate) enum DataFrameReadError {
    NotFound,
    InvalidCursor,
    InvalidPage(String),
    Storage(String),
}

impl std::fmt::Display for DataFrameReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "资源不存在"),
            Self::InvalidCursor => write!(formatter, "分页 cursor 无效"),
            Self::InvalidPage(message) => write!(formatter, "分页参数无效: {message}"),
            Self::Storage(message) => write!(formatter, "DataFrame 读取失败: {message}"),
        }
    }
}

impl std::error::Error for DataFrameReadError {}

#[derive(Clone)]
pub(crate) struct ParquetDataFrameReader {
    artifacts: ArtifactService,
}

impl ParquetDataFrameReader {
    pub(crate) fn new(artifacts: ArtifactService) -> Self {
        Self { artifacts }
    }

    pub(crate) async fn open_owned(
        &self,
        principal: &CurrentUser,
        artifact_id: i32,
    ) -> Result<File, DataFrameReadError> {
        let scope = self
            .artifacts
            .resolve_scope(principal.storage_scope_id)
            .await
            .map_err(|error| DataFrameReadError::Storage(error.to_string()))?;
        self.artifacts
            .open_owned(scope.id, artifact_id)
            .await
            .map_err(|error| match error {
                crate::application::ArtifactError::NotFound => DataFrameReadError::NotFound,
                other => DataFrameReadError::Storage(other.to_string()),
            })
    }

    pub(crate) async fn page(
        &self,
        principal: &CurrentUser,
        source: &DataFrameReadSource,
        limit: Option<usize>,
        cursor: Option<&str>,
        columns: &[String],
    ) -> Result<DataFramePageCore, DataFrameReadError> {
        let available = schema_columns(&source.schema_json)?;
        let limit = limit.unwrap_or(100);
        validate_projection(&available, columns, limit)?;
        let offset = match cursor {
            Some(cursor) => decode_cursor(cursor, &source.resource_key, &source.content_hash)?,
            None => 0,
        };
        let file = self.open_owned(principal, source.artifact_id).await?;
        let batches = ParquetInspector::projection_file(file, columns, offset, limit)
            .map_err(|error| DataFrameReadError::Storage(error.to_string()))?;
        let names = if let Some(batch) = batches.first() {
            batch
                .schema()
                .fields()
                .iter()
                .map(|field| field.name().clone())
                .collect()
        } else {
            page_column_names(columns, &source.schema_json)?
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
        let next_cursor = if offset.saturating_add(consumed)
            < usize::try_from(source.row_count).unwrap_or(usize::MAX)
        {
            Some(encode_cursor(
                &source.resource_key,
                &source.content_hash,
                offset + consumed,
            ))
        } else {
            None
        };
        Ok(DataFramePageCore {
            columns: names,
            rows,
            next_cursor,
            diagnostics,
        })
    }

    /// Scans every row of an owned Parquet artifact with a narrow projection.
    ///
    /// Unlike the public page API this primitive has no presentation row limit: the caller must
    /// aggregate inside `visit` and may not return raw model rows to the Agent boundary.
    pub(crate) async fn scan_projected<T, F>(
        &self,
        principal: &CurrentUser,
        source: &DataFrameReadSource,
        columns: &[String],
        mut state: T,
        mut visit: F,
    ) -> Result<T, DataFrameReadError>
    where
        T: Send + 'static,
        F: FnMut(&RecordBatch, &mut T) -> Result<(), DataFrameReadError> + Send + 'static,
    {
        let available = schema_columns(&source.schema_json)?;
        let mut seen = BTreeSet::new();
        for column in columns {
            if !available.contains(column) {
                return Err(DataFrameReadError::Storage(format!(
                    "模型 schema 缺少列: {column}"
                )));
            }
            if !seen.insert(column) {
                return Err(DataFrameReadError::Storage(format!("投影列重复: {column}")));
            }
        }
        let file = self.open_owned(principal, source.artifact_id).await?;
        let columns = columns.to_vec();
        tokio::task::spawn_blocking(move || {
            let mut builder = ParquetRecordBatchReaderBuilder::try_new(file)
                .map_err(|error| DataFrameReadError::Storage(error.to_string()))?;
            if !columns.is_empty() {
                for column in &columns {
                    builder.schema().index_of(column).map_err(|_| {
                        DataFrameReadError::Storage(format!("Parquet 缺少列: {column}"))
                    })?;
                }
                let projection = ProjectionMask::columns(
                    builder.parquet_schema(),
                    columns.iter().map(String::as_str),
                );
                builder = builder.with_projection(projection);
            }
            let reader = builder
                .build()
                .map_err(|error| DataFrameReadError::Storage(error.to_string()))?;
            for batch in reader {
                let batch =
                    batch.map_err(|error| DataFrameReadError::Storage(error.to_string()))?;
                visit(&batch, &mut state)?;
            }
            Ok(state)
        })
        .await
        .map_err(|error| DataFrameReadError::Storage(error.to_string()))?
    }

    pub(crate) async fn download(
        &self,
        principal: &CurrentUser,
        source: &DataFrameReadSource,
        file_name: String,
    ) -> Result<DataFrameDownload, DataFrameReadError> {
        Ok(DataFrameDownload {
            file: self.open_owned(principal, source.artifact_id).await?,
            file_name: safe_parquet_file_name(&file_name),
            media_type: "application/vnd.apache.parquet",
        })
    }
}

fn schema_columns(schema_json: &str) -> Result<Vec<String>, DataFrameReadError> {
    serde_json::from_str::<Vec<DataFrameColumn>>(schema_json)
        .map(|columns| columns.into_iter().map(|column| column.name).collect())
        .map_err(|error| DataFrameReadError::Storage(error.to_string()))
}

fn validate_projection(
    available: &[String],
    columns: &[String],
    limit: usize,
) -> Result<(), DataFrameReadError> {
    if !(1..=500).contains(&limit) {
        return Err(DataFrameReadError::InvalidPage(
            "limit 必须在 1..=500".into(),
        ));
    }
    let mut seen = BTreeSet::new();
    for column in columns {
        if !available.contains(column) {
            return Err(DataFrameReadError::InvalidPage(format!(
                "列不存在: {column}"
            )));
        }
        if !seen.insert(column) {
            return Err(DataFrameReadError::InvalidPage(format!("列重复: {column}")));
        }
    }
    Ok(())
}

fn page_column_names(
    columns: &[String],
    schema_json: &str,
) -> Result<Vec<String>, DataFrameReadError> {
    if columns.is_empty() {
        schema_columns(schema_json)
    } else {
        Ok(columns.to_vec())
    }
}

fn safe_parquet_file_name(name: &str) -> String {
    let safe = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect::<String>();
    format!("{safe}.parquet")
}

fn encode_cursor(resource_key: &str, content_hash: &str, offset: usize) -> String {
    hex(format!("v2|{resource_key}|{content_hash}|{offset}").as_bytes())
}

fn decode_cursor(
    encoded: &str,
    resource_key: &str,
    content_hash: &str,
) -> Result<usize, DataFrameReadError> {
    let bytes = unhex(encoded).ok_or(DataFrameReadError::InvalidCursor)?;
    let text = String::from_utf8(bytes).map_err(|_| DataFrameReadError::InvalidCursor)?;
    let parts = text.split('|').collect::<Vec<_>>();
    if parts.len() != 4 || parts[0] != "v2" || parts[1] != resource_key || parts[2] != content_hash
    {
        return Err(DataFrameReadError::InvalidCursor);
    }
    parts[3]
        .parse()
        .map_err(|_| DataFrameReadError::InvalidCursor)
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(value: &str) -> Option<Vec<u8>> {
    if !value.len().is_multiple_of(2) {
        return None;
    }
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).ok())
        .collect()
}

fn null(diagnostics: &mut BTreeMap<String, u64>, kind: &str) -> Value {
    *diagnostics.entry(kind.into()).or_default() += 1;
    Value::Null
}

pub(crate) fn value(
    array: &dyn Array,
    row: usize,
    diagnostics: &mut BTreeMap<String, u64>,
) -> Result<Value, DataFrameReadError> {
    if array.is_null(row) {
        return Ok(null(diagnostics, "missingValues"));
    }
    macro_rules! number {
        ($array:ty) => {
            if let Some(array) = array.as_any().downcast_ref::<$array>() {
                return Ok(Value::from(array.value(row)));
            }
        };
    }
    match array.data_type() {
        DataType::Utf8 => Ok(Value::String(
            array
                .as_any()
                .downcast_ref::<StringArray>()
                .expect("Utf8 array")
                .value(row)
                .into(),
        )),
        DataType::LargeUtf8 => Ok(Value::String(
            array
                .as_any()
                .downcast_ref::<LargeStringArray>()
                .expect("LargeUtf8 array")
                .value(row)
                .into(),
        )),
        DataType::Utf8View => Ok(Value::String(
            array
                .as_any()
                .downcast_ref::<StringViewArray>()
                .expect("Utf8View array")
                .value(row)
                .into(),
        )),
        DataType::Boolean => Ok(Value::Bool(
            array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .expect("Boolean array")
                .value(row),
        )),
        DataType::Int8 => {
            number!(Int8Array);
            unreachable!()
        }
        DataType::Int16 => {
            number!(Int16Array);
            unreachable!()
        }
        DataType::Int32 => {
            number!(Int32Array);
            unreachable!()
        }
        DataType::Int64 => {
            number!(Int64Array);
            unreachable!()
        }
        DataType::UInt8 => {
            number!(UInt8Array);
            unreachable!()
        }
        DataType::UInt16 => {
            number!(UInt16Array);
            unreachable!()
        }
        DataType::UInt32 => {
            number!(UInt32Array);
            unreachable!()
        }
        DataType::UInt64 => {
            let number = array
                .as_any()
                .downcast_ref::<UInt64Array>()
                .expect("UInt64 array")
                .value(row);
            Ok(serde_json::Number::from_u128(number as u128)
                .map(Value::Number)
                .unwrap_or_else(|| null(diagnostics, "outOfRangeValues")))
        }
        DataType::Float32 => {
            let number = array
                .as_any()
                .downcast_ref::<Float32Array>()
                .expect("Float32 array")
                .value(row) as f64;
            if number.is_finite() {
                Ok(Value::from(number))
            } else {
                Ok(null(diagnostics, "nonFiniteValues"))
            }
        }
        DataType::Float64 => {
            let number = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .expect("Float64 array")
                .value(row);
            if number.is_finite() {
                Ok(Value::from(number))
            } else {
                Ok(null(diagnostics, "nonFiniteValues"))
            }
        }
        DataType::List(_) => list_value(
            array
                .as_any()
                .downcast_ref::<ListArray>()
                .expect("List array")
                .value(row),
            diagnostics,
        ),
        DataType::LargeList(_) => list_value(
            array
                .as_any()
                .downcast_ref::<LargeListArray>()
                .expect("LargeList array")
                .value(row),
            diagnostics,
        ),
        DataType::ListView(_) => list_value(
            array
                .as_any()
                .downcast_ref::<ListViewArray>()
                .expect("ListView array")
                .value(row),
            diagnostics,
        ),
        DataType::LargeListView(_) => list_value(
            array
                .as_any()
                .downcast_ref::<LargeListViewArray>()
                .expect("LargeListView array")
                .value(row),
            diagnostics,
        ),
        DataType::FixedSizeList(_, _) => list_value(
            array
                .as_any()
                .downcast_ref::<FixedSizeListArray>()
                .expect("FixedSizeList array")
                .value(row),
            diagnostics,
        ),
        other => Err(DataFrameReadError::Storage(format!(
            "不支持的数据类型 {other}"
        ))),
    }
}

fn list_value(
    values: ArrayRef,
    diagnostics: &mut BTreeMap<String, u64>,
) -> Result<Value, DataFrameReadError> {
    (0..values.len())
        .map(|index| value(values.as_ref(), index, diagnostics))
        .collect::<Result<Vec<_>, _>>()
        .map(Value::Array)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use arrow_array::types::Float64Type;
    use arrow_array::{LargeListArray, StringViewArray};
    use serde_json::Value;

    use super::*;

    #[test]
    fn utf8_view_values_are_returned_as_json_strings() {
        let array = StringViewArray::from(vec![
            Some("bus-1"),
            None,
            Some("a string longer than twelve bytes"),
        ]);
        let mut diagnostics = BTreeMap::new();

        assert_eq!(
            value(&array, 0, &mut diagnostics).unwrap(),
            Value::String("bus-1".into())
        );
        assert_eq!(value(&array, 1, &mut diagnostics).unwrap(), Value::Null);
        assert_eq!(
            value(&array, 2, &mut diagnostics).unwrap(),
            Value::String("a string longer than twelve bytes".into())
        );
        assert_eq!(diagnostics.get("missingValues"), Some(&1));
    }

    #[test]
    fn cursor_is_bound_to_resource_kind_identity_hash_and_revision() {
        let cursor = encode_cursor("dataset:1", "schema-a", 25);

        assert_eq!(decode_cursor(&cursor, "dataset:1", "schema-a").unwrap(), 25);
        assert!(decode_cursor(&cursor, "model-dataframe:1", "schema-a").is_err());
        assert!(
            decode_cursor(&cursor, "draft-dataframe:1:draft:2:revision:4", "schema-a").is_err()
        );

        let draft = encode_cursor("draft-dataframe:1:draft:2:revision:3", "content-a", 10);
        assert!(
            decode_cursor(&draft, "draft-dataframe:1:draft:2:revision:4", "content-a").is_err()
        );
    }

    #[test]
    fn projection_rejects_unknown_and_duplicate_columns() {
        let available = vec!["id".to_string(), "value".to_string()];

        assert!(validate_projection(&available, &[], 1).is_ok());
        assert!(validate_projection(&available, &["id".into()], 500).is_ok());
        assert!(validate_projection(&available, &["missing".into()], 100).is_err());
        assert!(validate_projection(&available, &["id".into(), "id".into()], 100).is_err());
        assert!(validate_projection(&available, &[], 0).is_err());
        assert!(validate_projection(&available, &[], 501).is_err());
    }

    #[test]
    fn zero_rows_and_nested_list_values_have_stable_json_pages() {
        let schema_json = serde_json::json!([
            {"name":"id","dtype":"Utf8View","nullable":false,"unit":null},
            {"name":"values","dtype":"LargeList(Float64)","nullable":true,"unit":"pu"}
        ])
        .to_string();
        assert_eq!(
            page_column_names(&[], &schema_json).unwrap(),
            vec!["id", "values"]
        );

        let array = LargeListArray::from_iter_primitive::<Float64Type, _, _>([
            Some(vec![Some(1.0), None, Some(f64::NAN)]),
            None,
        ]);
        let mut diagnostics = BTreeMap::new();
        assert_eq!(
            value(&array, 0, &mut diagnostics).unwrap(),
            serde_json::json!([1.0, null, null])
        );
        assert_eq!(value(&array, 1, &mut diagnostics).unwrap(), Value::Null);
        assert_eq!(diagnostics.get("missingValues"), Some(&2));
        assert_eq!(diagnostics.get("nonFiniteValues"), Some(&1));
    }
}
