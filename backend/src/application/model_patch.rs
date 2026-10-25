use std::{collections::BTreeMap, io::Cursor, sync::Arc};

use arrow_array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray,
};
use arrow_schema::DataType;
use parquet::arrow::{ArrowWriter, arrow_reader::ParquetRecordBatchReaderBuilder};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::application::{ArtifactMetadata, ArtifactService, now_ms};
use crate::auth::CurrentUser;
use crate::db::models::DraftRef;
use crate::db::repositories::{VersionRepo, VersionRepoError};
use crate::storage::{ParquetInspector, StageLimits};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelPatch {
    pub row_key: BTreeMap<String, Value>,
    pub changes: BTreeMap<String, ModelChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelChange {
    pub from: Value,
    pub to: Value,
    pub unit: String,
}

#[derive(Debug)]
pub struct ModelMutationError {
    code: &'static str,
    message: String,
}
impl ModelMutationError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
}
impl std::fmt::Display for ModelMutationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for ModelMutationError {}

#[derive(Clone)]
pub struct ModelPatchService {
    versions: VersionRepo,
    artifacts: ArtifactService,
}

impl ModelPatchService {
    pub fn new(versions: VersionRepo, artifacts: ArtifactService) -> Self {
        Self {
            versions,
            artifacts,
        }
    }

    pub async fn modify(
        &self,
        principal: &CurrentUser,
        session_id: i32,
        table_name: String,
        patch: ModelPatch,
        expected: Option<DraftRef>,
    ) -> Result<DraftRef, ModelMutationError> {
        if patch.row_key.is_empty() || patch.changes.is_empty() {
            return Err(ModelMutationError::new(
                "invalid_model_patch",
                "rowKey 和 changes 不能为空",
            ));
        }
        validate_units(&patch)?;
        let workspace = self
            .versions
            .workspace(&principal.id, session_id, now_ms())
            .await
            .map_err(map_version)?;
        let artifact_id = match &workspace.draft {
            Some(draft) => draft
                .dataframes
                .iter()
                .find(|frame| frame.table_name == table_name)
                .map(|frame| frame.artifact_id),
            None => workspace
                .active
                .as_ref()
                .and_then(|version| {
                    version
                        .dataframes
                        .iter()
                        .find(|frame| frame.table_name == table_name)
                })
                .map(|frame| frame.artifact_id),
        }
        .ok_or_else(|| ModelMutationError::new("table_not_found", "DataFrame 不存在"))?;
        let scope = self
            .artifacts
            .resolve_scope(principal.storage_scope_id)
            .await
            .map_err(|error| ModelMutationError::new("artifact_error", error.to_string()))?;
        let artifact = self
            .artifacts
            .find_owned(scope.id, artifact_id)
            .await
            .map_err(|error| ModelMutationError::new("artifact_error", error.to_string()))?;
        let source = self
            .artifacts
            .store()
            .resolve(&artifact.storage_path)
            .map_err(|error| ModelMutationError::new("artifact_error", error.to_string()))?;
        let bytes = patch_parquet(&source, &patch)?;
        let staged = self
            .artifacts
            .store()
            .stage(Cursor::new(bytes), StageLimits::default())
            .map_err(|error| ModelMutationError::new("artifact_error", error.to_string()))?;
        let committed = self
            .artifacts
            .commit(scope.id, staged, ArtifactMetadata::parquet())
            .await
            .map_err(|error| ModelMutationError::new("artifact_error", error.to_string()))?;
        let committed_path = self
            .artifacts
            .store()
            .resolve(&committed.relative_path)
            .map_err(|error| ModelMutationError::new("artifact_error", error.to_string()))?;
        let metadata = ParquetInspector::metadata(&committed_path)
            .map_err(|error| ModelMutationError::new("invalid_model_patch", error.to_string()))?;
        let (schema_json, _) = crate::application::datasets::schema_json_and_hash(&metadata);
        let result = self
            .versions
            .replace_draft_dataframe(
                &principal.id,
                session_id,
                expected,
                table_name,
                committed.id,
                schema_json,
                metadata.row_count as i64,
                committed.sha256.clone(),
                now_ms(),
            )
            .await;
        match result {
            Ok((draft, replaced)) => {
                if let Some(replaced) = replaced {
                    self.artifacts.release(scope.id, replaced).await.ok();
                }
                Ok(draft)
            }
            Err(error) => {
                self.artifacts.release(scope.id, committed.id).await.ok();
                Err(map_version(error))
            }
        }
    }
}

fn map_version(error: VersionRepoError) -> ModelMutationError {
    match error {
        VersionRepoError::DraftConflict => {
            ModelMutationError::new("draft_revision_conflict", "草稿已变化，请重新读取工作区")
        }
        VersionRepoError::Repository(crate::db::repositories::RepoError::NotFound) => {
            ModelMutationError::new("not_found", "资源不存在")
        }
        other => ModelMutationError::new("workspace_error", format!("{other:?}")),
    }
}

fn validate_units(patch: &ModelPatch) -> Result<(), ModelMutationError> {
    for (column, change) in &patch.changes {
        if unit_dimension(&change.unit).is_none() {
            return Err(ModelMutationError::new(
                "unit_incompatible",
                format!("{column} 的单位 {} 不受支持", change.unit),
            ));
        }
        if change.from.is_number() != change.to.is_number()
            || change.from.is_string() != change.to.is_string()
            || change.from.is_boolean() != change.to.is_boolean()
        {
            return Err(ModelMutationError::new(
                "invalid_model_patch",
                format!("{column} 的 from/to 类型不一致"),
            ));
        }
    }
    Ok(())
}

fn unit_dimension(unit: &str) -> Option<&'static str> {
    match unit.trim().to_ascii_lowercase().as_str() {
        "w" | "kw" | "mw" => Some("power"),
        "var" | "kvar" | "mvar" => Some("reactive_power"),
        "v" | "kv" => Some("voltage"),
        "a" | "ka" => Some("current"),
        "ohm" | "ω" => Some("resistance"),
        "rad" | "degree" | "deg" => Some("angle"),
        "s" | "min" | "h" => Some("time"),
        "1" | "pu" | "%" => Some("dimensionless"),
        _ => None,
    }
}

fn patch_parquet(
    path: &std::path::Path,
    patch: &ModelPatch,
) -> Result<Vec<u8>, ModelMutationError> {
    let file = std::fs::File::open(path)
        .map_err(|error| ModelMutationError::new("invalid_model_patch", error.to_string()))?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|error| ModelMutationError::new("invalid_model_patch", error.to_string()))?;
    let schema = builder.schema().clone();
    let batches: Vec<RecordBatch> = builder
        .build()
        .map_err(|error| ModelMutationError::new("invalid_model_patch", error.to_string()))?
        .collect::<Result<_, _>>()
        .map_err(|error| ModelMutationError::new("invalid_model_patch", error.to_string()))?;
    let mut found = Vec::new();
    for (batch_index, batch) in batches.iter().enumerate() {
        for row in 0..batch.num_rows() {
            if patch.row_key.iter().all(|(name, expected)| {
                schema
                    .index_of(name)
                    .ok()
                    .and_then(|column| scalar_at(batch.column(column), row))
                    .is_some_and(|actual| actual == *expected)
            }) {
                found.push((batch_index, row));
            }
        }
    }
    if found.len() != 1 {
        return Err(ModelMutationError::new(
            "row_not_unique",
            format!("rowKey 匹配 {} 行", found.len()),
        ));
    }
    let (target_batch, target_row) = found[0];
    let mut output_batches = Vec::new();
    for (batch_index, batch) in batches.into_iter().enumerate() {
        if batch_index != target_batch {
            output_batches.push(batch);
            continue;
        }
        let mut columns = batch.columns().to_vec();
        for (name, change) in &patch.changes {
            let index = schema.index_of(name).map_err(|_| {
                ModelMutationError::new("invalid_model_patch", format!("列不存在: {name}"))
            })?;
            let actual = scalar_at(&columns[index], target_row).ok_or_else(|| {
                ModelMutationError::new("invalid_model_patch", "不支持的列类型或 null")
            })?;
            if actual != change.from {
                return Err(ModelMutationError::new(
                    "old_value_mismatch",
                    format!("{name} 的旧值不符"),
                ));
            }
            columns[index] = replace_scalar(&columns[index], target_row, &change.to)?;
        }
        output_batches.push(
            RecordBatch::try_new(schema.clone(), columns).map_err(|error| {
                ModelMutationError::new("invalid_model_patch", error.to_string())
            })?,
        );
    }
    let mut output = Vec::new();
    let properties = parquet::file::properties::WriterProperties::builder()
        .set_compression(parquet::basic::Compression::SNAPPY)
        .build();
    let mut writer = ArrowWriter::try_new(&mut output, schema, Some(properties))
        .map_err(|error| ModelMutationError::new("invalid_model_patch", error.to_string()))?;
    for batch in output_batches {
        writer
            .write(&batch)
            .map_err(|error| ModelMutationError::new("invalid_model_patch", error.to_string()))?;
    }
    writer
        .close()
        .map_err(|error| ModelMutationError::new("invalid_model_patch", error.to_string()))?;
    Ok(output)
}

fn scalar_at(array: &ArrayRef, row: usize) -> Option<Value> {
    if array.is_null(row) {
        return None;
    }
    match array.data_type() {
        DataType::Utf8 => Some(json!(
            array.as_any().downcast_ref::<StringArray>()?.value(row)
        )),
        DataType::Int64 => Some(json!(
            array.as_any().downcast_ref::<Int64Array>()?.value(row)
        )),
        DataType::Float64 => {
            serde_json::Number::from_f64(array.as_any().downcast_ref::<Float64Array>()?.value(row))
                .map(Value::Number)
        }
        DataType::Boolean => Some(json!(
            array.as_any().downcast_ref::<BooleanArray>()?.value(row)
        )),
        _ => None,
    }
}

fn replace_scalar(
    array: &ArrayRef,
    row: usize,
    value: &Value,
) -> Result<ArrayRef, ModelMutationError> {
    let invalid = || ModelMutationError::new("invalid_model_patch", "to 值与列类型不兼容");
    match array.data_type() {
        DataType::Utf8 => {
            let old = array
                .as_any()
                .downcast_ref::<StringArray>()
                .ok_or_else(invalid)?;
            let new = value.as_str().ok_or_else(invalid)?;
            Ok(Arc::new(StringArray::from(
                (0..old.len())
                    .map(|index| {
                        if index == row {
                            Some(new)
                        } else if old.is_null(index) {
                            None
                        } else {
                            Some(old.value(index))
                        }
                    })
                    .collect::<Vec<_>>(),
            )) as ArrayRef)
        }
        DataType::Int64 => {
            let old = array
                .as_any()
                .downcast_ref::<Int64Array>()
                .ok_or_else(invalid)?;
            let new = value.as_i64().ok_or_else(invalid)?;
            Ok(Arc::new(Int64Array::from(
                (0..old.len())
                    .map(|index| {
                        if index == row {
                            Some(new)
                        } else if old.is_null(index) {
                            None
                        } else {
                            Some(old.value(index))
                        }
                    })
                    .collect::<Vec<_>>(),
            )) as ArrayRef)
        }
        DataType::Float64 => {
            let old = array
                .as_any()
                .downcast_ref::<Float64Array>()
                .ok_or_else(invalid)?;
            let new = value.as_f64().ok_or_else(invalid)?;
            Ok(Arc::new(Float64Array::from(
                (0..old.len())
                    .map(|index| {
                        if index == row {
                            Some(new)
                        } else if old.is_null(index) {
                            None
                        } else {
                            Some(old.value(index))
                        }
                    })
                    .collect::<Vec<_>>(),
            )) as ArrayRef)
        }
        DataType::Boolean => {
            let old = array
                .as_any()
                .downcast_ref::<BooleanArray>()
                .ok_or_else(invalid)?;
            let new = value.as_bool().ok_or_else(invalid)?;
            Ok(Arc::new(BooleanArray::from(
                (0..old.len())
                    .map(|index| {
                        if index == row {
                            Some(new)
                        } else if old.is_null(index) {
                            None
                        } else {
                            Some(old.value(index))
                        }
                    })
                    .collect::<Vec<_>>(),
            )) as ArrayRef)
        }
        _ => Err(invalid()),
    }
}
