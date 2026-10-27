use std::io::Cursor;
use std::sync::Arc;

use arrow_array::{
    ArrayRef, BooleanArray, Float64Array, Int64Array, RecordBatch, StringArray, UInt32Array,
};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::auth::CurrentUser;
use crate::catalog::DataFrameSchemaContract;
use crate::db::repositories::UploadRepo;
use crate::storage::StageLimits;

use super::{
    CandidateDecision, CandidatePurpose, DataResourceService, DataResourceView,
    StudyConfigurationError, UploadBatchService, UploadFile, UploadSourceKind, now_ms,
};

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ColumnMapping {
    pub source: String,
    pub target: String,
}

#[derive(Debug, Clone)]
pub struct CreateMappedResourceRequest {
    pub session_id: i32,
    pub source_resource_id: String,
    pub display_name: String,
    pub alias: String,
    pub target_schema_ref: String,
    pub mappings: Vec<ColumnMapping>,
}

#[derive(Clone)]
pub struct DataMappingService {
    resources: DataResourceService,
    uploads: UploadBatchService,
    upload_repo: UploadRepo,
}

impl DataMappingService {
    pub fn new(
        resources: DataResourceService,
        uploads: UploadBatchService,
        upload_repo: UploadRepo,
    ) -> Self {
        Self {
            resources,
            uploads,
            upload_repo,
        }
    }

    pub async fn create(
        &self,
        user: &CurrentUser,
        request: CreateMappedResourceRequest,
        target: &DataFrameSchemaContract,
    ) -> Result<DataResourceView, StudyConfigurationError> {
        let source = self
            .resources
            .get(user, &request.source_resource_id)
            .await
            .map_err(map_resource_error)?;
        if source.session_id != request.session_id.to_string() {
            return Err(invalid("study_mapping_invalid", "映射源资源不属于当前会话"));
        }
        validate_mapping(target, &request.mappings)?;
        let source_columns = request
            .mappings
            .iter()
            .map(|mapping| mapping.source.clone())
            .collect::<Vec<_>>();
        let mut mapped_rows = Vec::new();
        let mut cursor = None;
        loop {
            let page = self
                .resources
                .rows(
                    user,
                    &request.source_resource_id,
                    Some(500),
                    cursor.as_deref(),
                    source_columns.clone(),
                )
                .await
                .map_err(map_resource_error)?;
            for row in page.rows {
                let mut mapped = serde_json::Map::new();
                for mapping in &request.mappings {
                    let index = page
                        .columns
                        .iter()
                        .position(|column| column == &mapping.source)
                        .expect("projection contains every requested source column");
                    mapped.insert(mapping.target.clone(), row[index].clone());
                }
                mapped_rows.push(mapped);
            }
            cursor = page.next_cursor;
            if cursor.is_none() {
                break;
            }
        }
        let bytes = mapped_parquet(target, &request.mappings, &mapped_rows)?;
        let fingerprint = hex::encode(Sha256::digest(&bytes));
        let client_id = format!("mapping-{}-{}", now_ms(), &fingerprint[..12]);
        let staged = self
            .resources
            .artifact_store()
            .stage(Cursor::new(bytes), StageLimits::default())
            .map_err(|error| invalid("study_mapping_invalid", &error.to_string()))?;
        let batch = self
            .uploads
            .accept(
                user,
                request.session_id,
                client_id.clone(),
                UploadSourceKind::Upload,
                vec![UploadFile {
                    client_file_id: client_id,
                    file_name: format!("{}.parquet", request.alias),
                    relative_path: None,
                    media_type: Some("application/vnd.apache.parquet".into()),
                    staged,
                }],
            )
            .await
            .map_err(|error| invalid("study_mapping_invalid", &error.to_string()))?;
        let candidate = batch
            .candidates
            .first()
            .ok_or_else(|| invalid("study_mapping_invalid", "映射没有产生候选 DataFrame"))?;
        self.upload_repo
            .set_controlled_mapping_schema(
                &user.id,
                batch.id.parse().expect("upload batch id is numeric"),
                &candidate.id,
                &request.target_schema_ref,
                serde_json::json!({
                    "source":"controlled_mapping",
                    "targetSchemaValidated":true,
                    "sourceResourceId": request.source_resource_id,
                    "targetSchemaRef": request.target_schema_ref,
                    "mappings": request.mappings,
                })
                .to_string(),
                now_ms(),
            )
            .await
            .map_err(StudyConfigurationError::from)?;
        let confirmation = self
            .resources
            .publish_candidates(
                user,
                batch.id.parse().expect("upload batch id is numeric"),
                vec![CandidateDecision {
                    candidate_id: candidate.id.clone(),
                    purpose: CandidatePurpose::DataResource,
                    selected: true,
                    table_name: None,
                    display_name: Some(request.display_name),
                    alias: Some(request.alias),
                }],
            )
            .await
            .map_err(map_resource_error)?;
        let resource = confirmation
            .resources
            .into_iter()
            .next()
            .ok_or_else(|| invalid("study_mapping_invalid", "映射资源发布失败"))?;
        if resource.declared_schema_ref.as_deref() != Some(request.target_schema_ref.as_str()) {
            return Err(invalid(
                "study_mapping_incompatible",
                "列映射后的类型仍不满足目标 schema",
            ));
        }
        self.resources
            .get(user, &resource.id)
            .await
            .map_err(map_resource_error)
    }
}

fn validate_mapping(
    target: &DataFrameSchemaContract,
    mappings: &[ColumnMapping],
) -> Result<(), StudyConfigurationError> {
    let targets = mappings
        .iter()
        .map(|mapping| mapping.target.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    if targets.len() != mappings.len()
        || mappings
            .iter()
            .any(|mapping| mapping.source.trim().is_empty())
        || target
            .fields
            .iter()
            .any(|field| field.required && !targets.contains(field.name.as_str()))
        || targets
            .iter()
            .any(|name| !target.fields.iter().any(|field| field.name == *name))
    {
        return Err(invalid(
            "study_mapping_invalid",
            "列映射必须唯一覆盖目标 schema 的全部必填字段",
        ));
    }
    Ok(())
}

fn mapped_parquet(
    contract: &DataFrameSchemaContract,
    mappings: &[ColumnMapping],
    rows: &[serde_json::Map<String, Value>],
) -> Result<Vec<u8>, StudyConfigurationError> {
    let mut fields = Vec::new();
    let mut arrays = Vec::<ArrayRef>::new();
    for mapping in mappings {
        let field = contract
            .fields
            .iter()
            .find(|field| field.name == mapping.target)
            .expect("mapping targets validated against schema");
        let (dtype, array) = mapped_array(&field.dtype, rows, &field.name, field.nullable)?;
        fields.push(Field::new(&field.name, dtype, field.nullable));
        arrays.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), arrays)
        .map_err(|error| invalid("study_mapping_invalid", &error.to_string()))?;
    let mut output = Vec::new();
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut writer = ArrowWriter::try_new(&mut output, schema, Some(properties))
        .map_err(|error| invalid("study_mapping_invalid", &error.to_string()))?;
    writer
        .write(&batch)
        .and_then(|_| writer.close().map(|_| ()))
        .map_err(|error| invalid("study_mapping_invalid", &error.to_string()))?;
    Ok(output)
}

fn mapped_array(
    dtype: &crate::catalog::DataTypeContract,
    rows: &[serde_json::Map<String, Value>],
    name: &str,
    nullable: bool,
) -> Result<(DataType, ArrayRef), StudyConfigurationError> {
    let values = rows.iter().map(|row| row.get(name)).collect::<Vec<_>>();
    if !nullable
        && values
            .iter()
            .any(|value| value.is_none_or(|value| value.is_null()))
    {
        return Err(invalid(
            "study_mapping_invalid",
            &format!("字段 {name} 不能为空"),
        ));
    }
    let kind = match dtype {
        crate::catalog::DataTypeContract::Primitive { name }
        | crate::catalog::DataTypeContract::Semantic { name } => name.as_str(),
        crate::catalog::DataTypeContract::Constructor { .. } => {
            return Err(invalid(
                "study_mapping_invalid",
                &format!("字段 {name} 的复合类型暂不支持列映射"),
            ));
        }
    };
    if kind == "str" || kind.contains("string") {
        let values = values
            .into_iter()
            .map(|value| value.filter(|value| !value.is_null()).map(value_as_string))
            .collect::<Vec<_>>();
        return Ok((DataType::Utf8, Arc::new(StringArray::from(values))));
    }
    if kind == "bool" {
        let values = values
            .into_iter()
            .map(|value| {
                value
                    .filter(|value| !value.is_null())
                    .map(parse_bool)
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok((DataType::Boolean, Arc::new(BooleanArray::from(values))));
    }
    if kind == "f64" || kind.contains("float") {
        let values = values
            .into_iter()
            .map(|value| {
                value
                    .filter(|value| !value.is_null())
                    .map(parse_f64)
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok((DataType::Float64, Arc::new(Float64Array::from(values))));
    }
    if kind == "u32" || kind.contains("u32") {
        let values = values
            .into_iter()
            .map(|value| {
                value
                    .filter(|value| !value.is_null())
                    .map(parse_u32)
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok((DataType::UInt32, Arc::new(UInt32Array::from(values))));
    }
    if kind.starts_with('i') || kind.contains("int") {
        let values = values
            .into_iter()
            .map(|value| {
                value
                    .filter(|value| !value.is_null())
                    .map(parse_i64)
                    .transpose()
            })
            .collect::<Result<Vec<_>, _>>()?;
        return Ok((DataType::Int64, Arc::new(Int64Array::from(values))));
    }
    Err(invalid(
        "study_mapping_invalid",
        &format!("字段 {name} 的类型 {kind} 暂不支持列映射"),
    ))
}

fn value_as_string(value: &Value) -> String {
    value
        .as_str()
        .map(str::to_string)
        .unwrap_or_else(|| value.to_string())
}
fn parse_f64(value: &Value) -> Result<f64, StudyConfigurationError> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.parse().ok())
        .ok_or_else(|| invalid("study_mapping_invalid", "映射值不能转换为 f64"))
}
fn parse_i64(value: &Value) -> Result<i64, StudyConfigurationError> {
    value
        .as_i64()
        .or_else(|| value.as_str()?.parse().ok())
        .ok_or_else(|| invalid("study_mapping_invalid", "映射值不能转换为 i64"))
}
fn parse_u32(value: &Value) -> Result<u32, StudyConfigurationError> {
    value
        .as_u64()
        .or_else(|| value.as_str()?.parse().ok())
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invalid("study_mapping_invalid", "映射值不能转换为 u32"))
}
fn parse_bool(value: &Value) -> Result<bool, StudyConfigurationError> {
    value
        .as_bool()
        .or_else(|| value.as_str()?.parse().ok())
        .ok_or_else(|| invalid("study_mapping_invalid", "映射值不能转换为 bool"))
}

fn invalid(code: &'static str, message: &str) -> StudyConfigurationError {
    StudyConfigurationError::Invalid(code, message.into())
}

fn map_resource_error(error: super::DataResourceError) -> StudyConfigurationError {
    if error.code() == "not_found" {
        StudyConfigurationError::NotFound
    } else {
        invalid("study_mapping_invalid", &error.to_string())
    }
}
