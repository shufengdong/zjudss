use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::sync::Arc;

use arrow_array::types::Float64Type;
use arrow_array::{
    Array, ArrayRef, BooleanArray, Float64Array, Int64Array, ListArray, RecordBatch, StringArray,
    UInt32Array,
};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::auth::{CurrentUser, UserId};
use crate::catalog::{DataFrameContractKind, DataFrameSchemaContract, DataTypeContract};
use crate::db::repositories::{
    DataResourceRepo, DeletionImpactRow, DeletionPreviewInput, PromoteDatasetInput, RepoError,
    ResourcePageRow, RetryStructOverlayRequest, StructuredPreviewConfirmation,
    StructuredPreviewInput, WorkflowStructOverlayMaterial,
};
use crate::engine_resources::EngineResourceSnapshot;
use crate::storage::StageLimits;

use super::dataframe_reader::{DataFrameReadError, DataFrameReadSource, ParquetDataFrameReader};
use super::opaque_token::{generate_token, hash_token, validate_presented_token};
use super::resource_alias::{
    AliasError, AliasPurpose, ResourceAliasPolicy, default_alias, normalize_alias,
};
use super::upload_confirmation::ReplacementAction;
use super::{
    ArtifactError, ArtifactMetadata, ArtifactService, CandidatePurpose, UploadBatchError, now_ms,
};

#[path = "study_struct_overlays.rs"]
pub(super) mod study_struct_overlays;
use self::study_struct_overlays::materialize_struct_overlay;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataResourceView {
    pub id: String,
    pub session_id: String,
    pub display_name: String,
    pub alias: String,
    pub status: String,
    pub declared_schema_ref: Option<String>,
    pub source_kind: String,
    pub schema: Value,
    pub schema_hash: String,
    pub content_hash: String,
    pub row_count: i64,
    pub size_bytes: i64,
    pub original_available: bool,
    pub source_batch_id: Option<String>,
    pub source_model_version_id: Option<String>,
    pub replaced_by_resource_id: Option<String>,
    pub lineage: Value,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataResourcePage {
    pub resources: Vec<DataResourceView>,
    pub next_cursor: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataResourceRows {
    pub resource_id: String,
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub next_cursor: Option<String>,
    pub diagnostics: BTreeMap<String, u64>,
}

pub struct DataResourceDownload {
    pub file: File,
    pub file_name: String,
    pub media_type: &'static str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataResourceDeletionPreview {
    pub impact_token: String,
    pub resource_id: String,
    pub display_name: String,
    pub alias: String,
    pub size_bytes: i64,
    pub pending_configuration_count: i64,
    pub frozen_reference_count: i64,
    pub historical_reference_count: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedDataResource {
    pub resource_id: String,
    pub tombstone: Value,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredReplacementPreview {
    pub preview_token: String,
    pub resource_id: String,
    pub expected_content_hash: String,
    pub schema_ref: String,
    pub differences: Vec<Value>,
    pub expires_at_ms: i64,
}

#[derive(Debug)]
pub enum DataResourceError {
    NotFound,
    Invalid(&'static str, String),
    Stale,
    Repository(RepoError),
    Artifact(ArtifactError),
    Upload(UploadBatchError),
    Read(&'static str, String),
    Io(std::io::Error),
}

impl DataResourceError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::Invalid(code, _) => code,
            Self::Stale => "resource_stale",
            Self::Repository(RepoError::NotFound) => "not_found",
            Self::Repository(RepoError::QuotaExceeded)
            | Self::Artifact(ArtifactError::QuotaExceeded) => "storage_quota_exceeded",
            Self::Repository(RepoError::InvalidState(message))
                if message == "deletion_impact_changed" =>
            {
                "deletion_impact_changed"
            }
            Self::Repository(RepoError::InvalidState(message))
                if message == "invalid_deletion_impact_token" =>
            {
                "invalid_deletion_impact_token"
            }
            Self::Repository(RepoError::InvalidState(message))
                if message == "resource_alias_conflict" =>
            {
                "resource_alias_conflict"
            }
            Self::Repository(RepoError::InvalidState(message)) if message == "resource_stale" => {
                "resource_stale"
            }
            Self::Repository(RepoError::InvalidState(message))
                if message == "draft_revision_conflict" =>
            {
                "draft_revision_conflict"
            }
            Self::Repository(RepoError::InvalidState(message))
                if message == "idempotency_conflict" =>
            {
                "idempotency_conflict"
            }
            Self::Repository(RepoError::InvalidState(message))
                if message == "upload_state_invalid" =>
            {
                "upload_state_invalid"
            }
            Self::Repository(RepoError::InvalidState(message))
                if message == "replacement_preview_invalid" =>
            {
                "replacement_preview_invalid"
            }
            Self::Upload(error) => match error.code() {
                "not_found" => "not_found",
                "upload_batch_expired" => "upload_batch_expired",
                "upload_candidate_invalid" => "upload_candidate_invalid",
                "upload_state_invalid" => "upload_state_invalid",
                "idempotency_conflict" => "idempotency_conflict",
                "storage_quota_exceeded" => "storage_quota_exceeded",
                _ => "upload_invalid",
            },
            Self::Read(code, _) => code,
            _ => "data_resource_error",
        }
    }
}

impl std::fmt::Display for DataResourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "资源不存在"),
            Self::Invalid(_, message) => write!(formatter, "{message}"),
            Self::Stale => write!(formatter, "资源内容已变化，请重新预览"),
            Self::Repository(error) => write!(formatter, "{error}"),
            Self::Artifact(error) => write!(formatter, "{error}"),
            Self::Upload(error) => write!(formatter, "{error}"),
            Self::Read(_, error) => write!(formatter, "{error}"),
            Self::Io(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for DataResourceError {}
impl From<RepoError> for DataResourceError {
    fn from(value: RepoError) -> Self {
        if matches!(value, RepoError::NotFound) {
            Self::NotFound
        } else {
            Self::Repository(value)
        }
    }
}
impl From<ArtifactError> for DataResourceError {
    fn from(value: ArtifactError) -> Self {
        Self::Artifact(value)
    }
}
impl From<UploadBatchError> for DataResourceError {
    fn from(value: UploadBatchError) -> Self {
        Self::Upload(value)
    }
}
impl From<DataFrameReadError> for DataResourceError {
    fn from(value: DataFrameReadError) -> Self {
        let code = match value {
            DataFrameReadError::InvalidCursor | DataFrameReadError::InvalidPage(_) => {
                "invalid_page"
            }
            DataFrameReadError::NotFound => "not_found",
            DataFrameReadError::TooManyRows(_) => "dataframe_read_failed",
            DataFrameReadError::Storage(_) => "dataframe_read_failed",
        };
        Self::Read(code, value.to_string())
    }
}
impl From<std::io::Error> for DataResourceError {
    fn from(value: std::io::Error) -> Self {
        Self::Io(value)
    }
}

#[derive(Clone)]
pub struct DataResourceService {
    repo: DataResourceRepo,
    artifacts: ArtifactService,
    reader: ParquetDataFrameReader,
    snapshot: Arc<EngineResourceSnapshot>,
}

impl DataResourceService {
    #[doc(hidden)]
    pub async fn materialize_study_struct_overlays(
        &self,
        user: &CurrentUser,
        prepared: &crate::application::PreparedStudyConfirmation,
    ) -> Result<Vec<WorkflowStructOverlayMaterial>, DataResourceError> {
        let mut materials: Vec<WorkflowStructOverlayMaterial> = Vec::new();
        for overlay in &prepared.struct_overlays {
            let result = async {
                let contract = self
                    .snapshot
                    .schemas
                    .resolve(&overlay.schema_ref)
                    .map_err(|error| Self::invalid("configuration_stale", &error.to_string()))?;
                if contract.kind != DataFrameContractKind::Struct {
                    return Err(Self::invalid(
                        "configuration_stale",
                        "研究参数 schema 不再是 Struct",
                    ));
                }
                let values = overlay.effective_values.as_object().ok_or_else(|| {
                    Self::invalid("structured_values_invalid", "Struct 参数必须是对象")
                })?;
                let source = if let Some(source_id) = overlay.source_resource_id.as_deref() {
                    let source = self.repo.find(&user.id, source_id, false).await?;
                    let artifact_id = source.resource.normalized_artifact_id.ok_or_else(|| {
                        Self::invalid("configuration_stale", "Struct 来源缺少 artifact")
                    })?;
                    let schema_ref = source.resource.declared_schema_ref.ok_or_else(|| {
                        Self::invalid("configuration_stale", "Struct 来源缺少 schema")
                    })?;
                    if source.resource.status != "live" || schema_ref != overlay.schema_ref {
                        return Err(Self::invalid(
                            "configuration_stale",
                            "Struct overlay 来源已经变化",
                        ));
                    }
                    Some((
                        source.resource.id,
                        artifact_id,
                        source.normalized_sha256,
                        source.resource.schema_hash,
                    ))
                } else {
                    None
                };
                let reusable_source = source
                    .as_ref()
                    .map(|(_, artifact_id, content_hash, _)| (*artifact_id, content_hash.as_str()));
                let stored = materialize_struct_overlay(
                    &self.artifacts,
                    user.storage_scope_id,
                    contract,
                    values,
                    reusable_source,
                )
                .await?;
                Ok(WorkflowStructOverlayMaterial {
                    input_id: overlay.input_id.clone(),
                    source_resource_id: source.as_ref().map(|value| value.0.clone()),
                    source_artifact_id: source.as_ref().map(|value| value.1),
                    source_content_hash: source.as_ref().map(|value| value.2.clone()),
                    source_schema_hash: source.as_ref().map(|value| value.3.clone()),
                    artifact_id: stored.artifact_id,
                    content_hash: stored.content_hash,
                    schema_ref: overlay.schema_ref.clone(),
                    schema_hash: contract.schema_hash.clone(),
                    effective_values_json: overlay.effective_values.to_string(),
                    user_overrides_json: overlay.user_overrides.to_string(),
                    selector_overrides_json: overlay.selector_overrides.to_string(),
                    overrides_json: overlay.overrides.to_string(),
                    owns_artifact_reference: stored.owns_artifact_reference,
                })
            }
            .await;
            match result {
                Ok(material) => materials.push(material),
                Err(error) => {
                    for material in &materials {
                        if material.owns_artifact_reference {
                            let _ = self
                                .artifacts
                                .release(user.storage_scope_id, material.artifact_id)
                                .await;
                        }
                    }
                    return Err(error);
                }
            }
        }
        Ok(materials)
    }

    #[doc(hidden)]
    pub async fn materialize_retry_struct_overlays(
        &self,
        user: &CurrentUser,
        requests: &[RetryStructOverlayRequest],
    ) -> Result<Vec<WorkflowStructOverlayMaterial>, DataResourceError> {
        let mut materials: Vec<WorkflowStructOverlayMaterial> = Vec::new();
        for request in requests {
            let result = async {
                let source = self
                    .repo
                    .find(&user.id, &request.source_resource_id, false)
                    .await?;
                if source.resource.normalized_artifact_id != Some(request.source_artifact_id)
                    || source.normalized_sha256 != request.source_content_hash
                    || source.resource.schema_hash != request.source_schema_hash
                    || source.resource.declared_schema_ref.as_deref()
                        != Some(request.schema_ref.as_str())
                {
                    return Err(Self::invalid(
                        "configuration_stale",
                        "Struct overlay 来源已经变化",
                    ));
                }
                let page = self
                    .rows(user, &request.source_resource_id, Some(1), None, Vec::new())
                    .await?;
                let row = page.rows.first().ok_or_else(|| {
                    Self::invalid("structured_values_invalid", "Struct 参数缺少数据行")
                })?;
                let mut values = page
                    .columns
                    .iter()
                    .cloned()
                    .zip(row.iter().cloned())
                    .collect::<serde_json::Map<_, _>>();
                let overrides = request.overrides.as_object().ok_or_else(|| {
                    Self::invalid("structured_values_invalid", "Struct 覆盖必须是对象")
                })?;
                for (field, value) in overrides {
                    values.insert(field.clone(), value.clone());
                }
                let contract = self
                    .snapshot
                    .schemas
                    .resolve(&request.schema_ref)
                    .map_err(|error| Self::invalid("configuration_stale", &error.to_string()))?;
                if contract.kind != DataFrameContractKind::Struct {
                    return Err(Self::invalid(
                        "configuration_stale",
                        "研究参数 schema 不再是 Struct",
                    ));
                }
                let stored = materialize_struct_overlay(
                    &self.artifacts,
                    user.storage_scope_id,
                    contract,
                    &values,
                    Some((request.source_artifact_id, &request.source_content_hash)),
                )
                .await?;
                Ok(WorkflowStructOverlayMaterial {
                    input_id: request.input_id.clone(),
                    source_resource_id: Some(request.source_resource_id.clone()),
                    source_artifact_id: Some(request.source_artifact_id),
                    source_content_hash: Some(request.source_content_hash.clone()),
                    source_schema_hash: Some(request.source_schema_hash.clone()),
                    artifact_id: stored.artifact_id,
                    content_hash: stored.content_hash,
                    schema_ref: request.schema_ref.clone(),
                    schema_hash: contract.schema_hash.clone(),
                    effective_values_json: Value::Object(values).to_string(),
                    user_overrides_json: request.user_overrides.to_string(),
                    selector_overrides_json: request.selector_overrides.to_string(),
                    overrides_json: request.overrides.to_string(),
                    owns_artifact_reference: stored.owns_artifact_reference,
                })
            }
            .await;
            match result {
                Ok(material) => materials.push(material),
                Err(error) => {
                    for material in &materials {
                        if material.owns_artifact_reference {
                            let _ = self
                                .artifacts
                                .release(user.storage_scope_id, material.artifact_id)
                                .await;
                        }
                    }
                    return Err(error);
                }
            }
        }
        Ok(materials)
    }

    pub(crate) fn artifact_store(&self) -> &crate::storage::ArtifactStore {
        self.artifacts.store()
    }

    pub fn new(
        repo: DataResourceRepo,
        artifacts: ArtifactService,
        snapshot: Arc<EngineResourceSnapshot>,
    ) -> Self {
        Self {
            repo,
            reader: ParquetDataFrameReader::new(artifacts.clone()),
            artifacts,
            snapshot,
        }
    }

    pub async fn revalidate_user_aliases(&self, user: &UserId) -> Result<usize, DataResourceError> {
        let resources = self.repo.list_current_for_user(user).await?;
        let mut marked = 0;
        for resource in resources {
            if resource.status == "live"
                && matches!(
                    self.validate_alias(
                        &resource.alias,
                        CandidatePurpose::DataResource,
                        resource.declared_schema_ref.as_deref(),
                    ),
                    Err(DataResourceError::Invalid("resource_alias_reserved", _))
                )
            {
                self.repo
                    .mark_needs_rename(user, &resource.id, now_ms())
                    .await?;
                marked += 1;
            }
        }
        Ok(marked)
    }

    pub async fn list(
        &self,
        user: &CurrentUser,
        session_id: i32,
        cursor: Option<String>,
        limit: u16,
    ) -> Result<DataResourcePage, DataResourceError> {
        if !(1..=100).contains(&limit) {
            return Err(Self::invalid("invalid_page", "limit 必须在 1..=100"));
        }
        let before = cursor.as_deref().map(decode_cursor).transpose()?;
        let mut rows = self
            .repo
            .list(&user.id, session_id, before, i64::from(limit) + 1)
            .await?;
        for row in &mut rows {
            self.revalidate_alias(user, row).await?;
        }
        let has_more = rows.len() > usize::from(limit);
        let selected = rows
            .into_iter()
            .take(usize::from(limit))
            .collect::<Vec<_>>();
        let next_cursor = has_more
            .then(|| {
                selected
                    .last()
                    .map(|row| encode_cursor(row.resource.created_at_ms, &row.resource.id))
            })
            .flatten();
        Ok(DataResourcePage {
            resources: selected.into_iter().map(view).collect(),
            next_cursor,
        })
    }

    pub async fn get(
        &self,
        user: &CurrentUser,
        id: &str,
    ) -> Result<DataResourceView, DataResourceError> {
        let mut row = self.repo.find(&user.id, id, false).await?;
        self.revalidate_alias(user, &mut row).await?;
        Ok(view(row))
    }
    pub async fn historical(
        &self,
        user: &CurrentUser,
        id: &str,
    ) -> Result<DataResourceView, DataResourceError> {
        Ok(view(self.repo.find(&user.id, id, true).await?))
    }

    pub async fn rename_alias(
        &self,
        user: &CurrentUser,
        id: &str,
        alias: &str,
    ) -> Result<DataResourceView, DataResourceError> {
        let current = self.repo.find(&user.id, id, false).await?;
        self.validate_alias(
            alias,
            CandidatePurpose::DataResource,
            current.resource.declared_schema_ref.as_deref(),
        )?;
        self.repo
            .rename(
                &user.id,
                id,
                None,
                Some((alias.trim().into(), normalize_alias(alias))),
                now_ms(),
            )
            .await?;
        self.get(user, id).await
    }

    pub async fn promote_dataset(
        &self,
        user: &CurrentUser,
        session_id: i32,
        dataset_id: i32,
        display_name: String,
    ) -> Result<DataResourceView, DataResourceError> {
        let display_name = display_name.trim();
        if display_name.is_empty() {
            return Err(Self::invalid(
                "data_resource_invalid",
                "displayName 不能为空",
            ));
        }
        let resource_id =
            stable_resource_id(dataset_id, &format!("dataset:{}:{}", dataset_id, now_ms()));
        let alias = default_alias(display_name);
        let record = self
            .repo
            .promote_dataset(
                &user.id,
                PromoteDatasetInput {
                    session_id,
                    dataset_id,
                    resource_id,
                    display_name: display_name.into(),
                    alias: alias.clone(),
                    normalized_alias: normalize_alias(&alias),
                    now_ms: now_ms(),
                },
            )
            .await?;
        self.get(user, &record.id).await
    }

    pub async fn update(
        &self,
        user: &CurrentUser,
        id: &str,
        display_name: Option<String>,
        alias: Option<String>,
    ) -> Result<DataResourceView, DataResourceError> {
        if display_name
            .as_deref()
            .is_some_and(|name| name.trim().is_empty())
        {
            return Err(Self::invalid(
                "data_resource_invalid",
                "displayName 不能为空",
            ));
        }
        let current = self.repo.find(&user.id, id, false).await?;
        let alias = alias
            .map(|alias| -> Result<(String, String), DataResourceError> {
                self.validate_alias(
                    &alias,
                    CandidatePurpose::DataResource,
                    current.resource.declared_schema_ref.as_deref(),
                )?;
                Ok((alias.trim().into(), normalize_alias(&alias)))
            })
            .transpose()?;
        self.repo
            .rename(
                &user.id,
                id,
                display_name.map(|name| name.trim().to_string()),
                alias,
                now_ms(),
            )
            .await?;
        self.get(user, id).await
    }

    pub async fn rows(
        &self,
        user: &CurrentUser,
        id: &str,
        limit: Option<usize>,
        cursor: Option<&str>,
        columns: Vec<String>,
    ) -> Result<DataResourceRows, DataResourceError> {
        let row = self.repo.find(&user.id, id, false).await?;
        let source = read_source(&row);
        let page = self
            .reader
            .page(user, &source, limit, cursor, &columns)
            .await?;
        Ok(DataResourceRows {
            resource_id: id.into(),
            columns: page.columns,
            rows: page.rows,
            next_cursor: page.next_cursor,
            diagnostics: page.diagnostics,
        })
    }

    pub async fn download(
        &self,
        user: &CurrentUser,
        id: &str,
        original: bool,
    ) -> Result<DataResourceDownload, DataResourceError> {
        let row = self.repo.find(&user.id, id, false).await?;
        if original {
            let artifact = row
                .resource
                .source_artifact_id
                .ok_or_else(|| Self::invalid("original_unavailable", "该资源没有原文件"))?;
            let file = self
                .artifacts
                .open_owned(user.storage_scope_id, artifact)
                .await?;
            Ok(DataResourceDownload {
                file,
                file_name: safe_name(&row.resource.display_name, "original"),
                media_type: "application/octet-stream",
            })
        } else {
            let download = self
                .reader
                .download(user, &read_source(&row), row.resource.alias.clone())
                .await?;
            Ok(DataResourceDownload {
                file: download.file,
                file_name: download.file_name,
                media_type: download.media_type,
            })
        }
    }

    pub async fn preview_deletion(
        &self,
        user: &CurrentUser,
        id: &str,
    ) -> Result<DataResourceDeletionPreview, DataResourceError> {
        let impact = self.repo.deletion_impact(&user.id, id).await?;
        let value = impact_value(&impact);
        let impact_json = serde_json::to_string(&value).expect("impact serializable");
        let impact_hash = hex::encode(Sha256::digest(impact_json.as_bytes()));
        let token = generate_token();
        let token_hash = hash_token(&token);
        let expires = now_ms() + 10 * 60 * 1000;
        self.repo
            .store_deletion_preview(
                &user.id,
                DeletionPreviewInput {
                    resource_id: id.into(),
                    token_hash,
                    impact_hash,
                    impact_json,
                    expires_at_ms: expires,
                    created_at_ms: now_ms(),
                },
            )
            .await?;
        Ok(DataResourceDeletionPreview {
            impact_token: token,
            resource_id: id.into(),
            display_name: impact.resource.display_name,
            alias: impact.resource.alias,
            size_bytes: impact.normalized_size_bytes,
            pending_configuration_count: impact.pending_configuration_count,
            frozen_reference_count: impact.frozen_reference_count,
            historical_reference_count: impact.historical_reference_count,
            expires_at_ms: expires,
        })
    }

    pub async fn delete(
        &self,
        user: &CurrentUser,
        id: &str,
        impact_token: &str,
    ) -> Result<DeletedDataResource, DataResourceError> {
        let impact = match self.repo.deletion_impact(&user.id, id).await {
            Ok(value) => value,
            Err(RepoError::NotFound) => {
                let row = self.repo.find_record(&user.id, id).await?;
                if row.status == "deleted" {
                    return Ok(DeletedDataResource {
                        resource_id: id.into(),
                        tombstone: row
                            .tombstone_json
                            .and_then(|value| serde_json::from_str(&value).ok())
                            .unwrap_or_else(|| serde_json::json!({"id":id})),
                    });
                }
                return Err(DataResourceError::NotFound);
            }
            Err(error) => return Err(error.into()),
        };
        let impact_hash = hex::encode(Sha256::digest(
            serde_json::to_string(&impact_value(&impact))
                .unwrap()
                .as_bytes(),
        ));
        validate_presented_token(impact_token).map_err(|_| {
            Self::invalid(
                "invalid_deletion_impact_token",
                "删除影响确认凭据无效或已过期",
            )
        })?;
        let token_hash = hash_token(impact_token);
        let deleted = self
            .repo
            .delete(&user.id, id, token_hash, impact_hash, now_ms())
            .await?;
        Ok(DeletedDataResource {
            resource_id: id.into(),
            tombstone: serde_json::json!({"id":id,"alias":deleted.resource.alias,"schemaHash":deleted.resource.schema_hash}),
        })
    }

    pub async fn preview_structured_replacement(
        &self,
        user: &CurrentUser,
        id: &str,
        expected_content_hash: &str,
        schema_ref: &str,
        values: Value,
    ) -> Result<StructuredReplacementPreview, DataResourceError> {
        let current = self.repo.find(&user.id, id, false).await?;
        if current.normalized_sha256 != expected_content_hash {
            return Err(DataResourceError::Stale);
        }
        if current.resource.declared_schema_ref.as_deref() != Some(schema_ref) {
            return Err(Self::invalid(
                "structured_schema_mismatch",
                "资源声明的 Struct schema 与请求不一致",
            ));
        }
        let contract = self
            .snapshot
            .schemas
            .resolve(schema_ref)
            .map_err(|error| Self::invalid("structured_schema_mismatch", &error.to_string()))?;
        if contract.kind != DataFrameContractKind::Struct {
            return Err(Self::invalid(
                "structured_schema_mismatch",
                "schema 不是 Struct",
            ));
        }
        let object = values
            .as_object()
            .ok_or_else(|| Self::invalid("structured_values_invalid", "values 必须是对象"))?;
        let parquet = struct_parquet(contract, object)?;
        let staged = self
            .artifacts
            .store()
            .stage(std::io::Cursor::new(parquet), StageLimits::default())
            .map_err(ArtifactError::from)?;
        let artifact = self
            .artifacts
            .commit(user.storage_scope_id, staged, ArtifactMetadata::parquet())
            .await?;
        let columns = contract
            .fields
            .iter()
            .map(|field| super::dataframe_reader::DataFrameColumn {
                name: field.name.clone(),
                dtype: dtype_label(&field.dtype),
                nullable: field.nullable,
                unit: Some(field.unit.0.clone()),
            })
            .collect::<Vec<_>>();
        let schema_json = serde_json::to_string(&columns)
            .map_err(|error| Self::invalid("structured_values_invalid", &error.to_string()))?;
        let old_page = self
            .reader
            .page(user, &read_source(&current), Some(1), None, &[])
            .await?;
        let differences = contract
            .fields
            .iter()
            .enumerate()
            .filter_map(|(index, field)| {
                let before = old_page
                    .rows
                    .first()
                    .and_then(|row| row.get(index))
                    .cloned()
                    .unwrap_or(Value::Null);
                let after = object.get(&field.name).cloned().unwrap_or(Value::Null);
                (before != after)
                    .then(|| serde_json::json!({"field":field.name,"before":before,"after":after}))
            })
            .collect::<Vec<_>>();
        let now = now_ms();
        let pending_id = stable_resource_id(
            current.resource.session_id,
            &format!("structured:{id}:{now}:{}", artifact.sha256),
        );
        let preview_token = generate_token();
        let token_hash = hash_token(&preview_token);
        let impact_json = serde_json::to_string(&differences).unwrap();
        let impact_hash = hex::encode(Sha256::digest(impact_json.as_bytes()));
        let expires = now + 10 * 60 * 1000;
        let input = StructuredPreviewInput {
            token_hash,
            user_id: user.id.0.clone(),
            old_resource_id: id.into(),
            pending_artifact_id: artifact.id,
            pending_resource_id: pending_id,
            expected_content_hash: expected_content_hash.into(),
            schema_ref: schema_ref.into(),
            schema_json,
            schema_hash: contract.schema_hash.clone(),
            values_json: values.to_string(),
            display_name: current.resource.display_name.clone(),
            proposed_alias: current.resource.alias.clone(),
            impact_hash,
            impact_json,
            expires_at_ms: expires,
            created_at_ms: now,
        };
        match self.repo.store_structured_preview(input).await {
            Ok(()) => {}
            Err(error) => {
                let _ = self
                    .artifacts
                    .release(user.storage_scope_id, artifact.id)
                    .await;
                return Err(error.into());
            }
        }
        Ok(StructuredReplacementPreview {
            preview_token,
            resource_id: id.into(),
            expected_content_hash: expected_content_hash.into(),
            schema_ref: schema_ref.into(),
            differences,
            expires_at_ms: expires,
        })
    }

    pub async fn confirm_structured_replacement(
        &self,
        user: &CurrentUser,
        preview_token: &str,
        action: ReplacementAction,
    ) -> Result<Option<DataResourceView>, DataResourceError> {
        validate_presented_token(preview_token)
            .map_err(|_| Self::invalid("replacement_preview_invalid", "替换预览不存在或已过期"))?;
        let token_hash = hash_token(preview_token);
        let confirmation = self
            .repo
            .confirm_structured_preview(&user.id, token_hash, action.as_str().into(), now_ms())
            .await?;
        match confirmation {
            StructuredPreviewConfirmation::Published(resource) => {
                Ok(Some(self.get(user, &resource.id).await?))
            }
            StructuredPreviewConfirmation::Cancelled => Ok(None),
            StructuredPreviewConfirmation::Rejected { code } => Err(Self::invalid(
                code,
                if code == "resource_stale" {
                    "原资源已发生变化，请重新生成预览"
                } else {
                    "替换预览不存在或已过期"
                },
            )),
        }
    }

    pub async fn cleanup_expired_structured_previews(&self) -> Result<usize, DataResourceError> {
        self.repo
            .take_expired_structured_previews(now_ms())
            .await
            .map_err(Into::into)
    }

    pub(super) fn validate_alias(
        &self,
        alias: &str,
        purpose: CandidatePurpose,
        schema_ref: Option<&str>,
    ) -> Result<(), DataResourceError> {
        let purpose = if purpose == CandidatePurpose::ModelTable {
            AliasPurpose::ModelTable
        } else {
            AliasPurpose::DataResource
        };
        match ResourceAliasPolicy::new(&self.snapshot).validate(alias, purpose, schema_ref) {
            Ok(()) => Ok(()),
            Err(AliasError::Invalid) => Err(Self::invalid(
                "resource_alias_invalid",
                "alias 必须是合法 RSS 标识符",
            )),
            Err(AliasError::Reserved) => Err(Self::invalid(
                "resource_alias_reserved",
                "alias 与模型、函数、结果或临时命名空间冲突",
            )),
        }
    }

    async fn revalidate_alias(
        &self,
        user: &CurrentUser,
        row: &mut ResourcePageRow,
    ) -> Result<(), DataResourceError> {
        if row.resource.status != "live" {
            return Ok(());
        }
        if matches!(
            self.validate_alias(
                &row.resource.alias,
                CandidatePurpose::DataResource,
                row.resource.declared_schema_ref.as_deref(),
            ),
            Err(DataResourceError::Invalid("resource_alias_reserved", _))
        ) {
            self.repo
                .mark_needs_rename(&user.id, &row.resource.id, now_ms())
                .await?;
            row.resource.status = "needs_rename".into();
        }
        Ok(())
    }
    fn invalid(code: &'static str, message: &str) -> DataResourceError {
        DataResourceError::Invalid(code, message.into())
    }
}

pub(super) fn view(row: ResourcePageRow) -> DataResourceView {
    DataResourceView {
        id: row.resource.id,
        session_id: row.resource.session_id.to_string(),
        display_name: row.resource.display_name,
        alias: row.resource.alias,
        status: row.resource.status,
        declared_schema_ref: row.resource.declared_schema_ref,
        source_kind: row.resource.source_kind,
        schema: serde_json::from_str(&row.resource.schema_json).unwrap_or(Value::Null),
        schema_hash: row.resource.schema_hash,
        content_hash: row.normalized_sha256,
        row_count: row.resource.row_count,
        size_bytes: row.normalized_size_bytes,
        original_available: row.resource.source_artifact_id.is_some(),
        source_batch_id: row.resource.source_batch_id.map(|id| id.to_string()),
        source_model_version_id: row
            .resource
            .source_model_version_id
            .map(|id| id.to_string()),
        replaced_by_resource_id: row.resource.replaced_by_resource_id,
        lineage: serde_json::from_str(&row.resource.lineage_json).unwrap_or(Value::Null),
        created_at_ms: row.resource.created_at_ms,
    }
}
fn read_source(row: &ResourcePageRow) -> DataFrameReadSource {
    DataFrameReadSource {
        resource_key: format!("data-resource:{}", row.resource.id),
        artifact_id: row
            .resource
            .normalized_artifact_id
            .expect("live data resource has a normalized artifact"),
        schema_json: row.resource.schema_json.clone(),
        row_count: row.resource.row_count,
        content_hash: row.normalized_sha256.clone(),
    }
}
pub(super) fn stable_resource_id(batch: i32, candidate: &str) -> String {
    format!(
        "dr_{}",
        &hex::encode(Sha256::digest(format!("{batch}:{candidate}").as_bytes()))[..24]
    )
}
fn encode_cursor(created: i64, id: &str) -> String {
    hex::encode(format!("{created}\0{id}"))
}
fn decode_cursor(value: &str) -> Result<(i64, String), DataResourceError> {
    let bytes = hex::decode(value)
        .map_err(|_| DataResourceService::invalid("invalid_page", "cursor 无效"))?;
    let decoded = String::from_utf8(bytes)
        .map_err(|_| DataResourceService::invalid("invalid_page", "cursor 无效"))?;
    let (created, id) = decoded
        .split_once('\0')
        .ok_or_else(|| DataResourceService::invalid("invalid_page", "cursor 无效"))?;
    Ok((
        created
            .parse()
            .map_err(|_| DataResourceService::invalid("invalid_page", "cursor 无效"))?,
        id.into(),
    ))
}
fn safe_name(name: &str, fallback: &str) -> String {
    let safe = name.replace(['/', '\\', '\r', '\n'], "_");
    if safe.trim().is_empty() {
        fallback.into()
    } else {
        safe
    }
}
fn impact_value(impact: &DeletionImpactRow) -> Value {
    serde_json::json!({"resourceId":impact.resource.id,"schemaHash":impact.resource.schema_hash,"pendingConfigurationCount":impact.pending_configuration_count,"frozenReferenceCount":impact.frozen_reference_count,"historicalReferenceCount":impact.historical_reference_count})
}
pub(super) fn struct_parquet(
    contract: &DataFrameSchemaContract,
    values: &serde_json::Map<String, Value>,
) -> Result<Vec<u8>, DataResourceError> {
    let known = contract
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();
    if values.keys().any(|name| !known.contains(name.as_str())) {
        return Err(DataResourceService::invalid(
            "structured_values_invalid",
            "values 包含 schema 未声明字段",
        ));
    }
    let mut fields = Vec::new();
    let mut arrays = Vec::<ArrayRef>::new();
    for field in &contract.fields {
        let value = values.get(&field.name);
        if field.required && value.is_none_or(Value::is_null) {
            return Err(DataResourceService::invalid(
                "structured_values_invalid",
                &format!("缺少必填字段 {}", field.name),
            ));
        }
        let (dtype, array) = struct_array(&field.dtype, value, field.nullable, &field.name)?;
        fields.push(Field::new(&field.name, dtype, field.nullable));
        arrays.push(array);
    }
    let schema = Arc::new(Schema::new(fields));
    let batch = RecordBatch::try_new(schema.clone(), arrays).map_err(|error| {
        DataResourceService::invalid("structured_values_invalid", &error.to_string())
    })?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut output = Vec::new();
    let mut writer =
        ArrowWriter::try_new(&mut output, schema, Some(properties)).map_err(|error| {
            DataResourceService::invalid("structured_values_invalid", &error.to_string())
        })?;
    writer
        .write(&batch)
        .and_then(|_| writer.close().map(|_| ()))
        .map_err(|error| {
            DataResourceService::invalid("structured_values_invalid", &error.to_string())
        })?;
    Ok(output)
}
fn struct_array(
    dtype: &DataTypeContract,
    value: Option<&Value>,
    nullable: bool,
    name: &str,
) -> Result<(DataType, ArrayRef), DataResourceError> {
    let null = value.is_none_or(Value::is_null);
    if null && !nullable {
        return Err(DataResourceService::invalid(
            "structured_values_invalid",
            &format!("字段 {name} 不能为空"),
        ));
    }
    match dtype {
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if kind == "str" || kind.contains("string") =>
        {
            let item = if null {
                None
            } else {
                Some(value.unwrap().as_str().ok_or_else(|| {
                    DataResourceService::invalid(
                        "structured_values_invalid",
                        &format!("字段 {name} 必须是字符串"),
                    )
                })?)
            };
            Ok((DataType::Utf8, Arc::new(StringArray::from(vec![item]))))
        }
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if kind == "bool" =>
        {
            let item = if null {
                None
            } else {
                Some(value.unwrap().as_bool().ok_or_else(|| {
                    DataResourceService::invalid(
                        "structured_values_invalid",
                        &format!("字段 {name} 必须是布尔值"),
                    )
                })?)
            };
            Ok((DataType::Boolean, Arc::new(BooleanArray::from(vec![item]))))
        }
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if kind == "f64" || kind.contains("float") =>
        {
            let item = if null {
                None
            } else {
                Some(value.unwrap().as_f64().ok_or_else(|| {
                    DataResourceService::invalid(
                        "structured_values_invalid",
                        &format!("字段 {name} 必须是数值"),
                    )
                })?)
            };
            Ok((DataType::Float64, Arc::new(Float64Array::from(vec![item]))))
        }
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if kind == "u32" || kind.contains("u32") =>
        {
            let item = if null {
                None
            } else {
                Some(
                    u32::try_from(value.unwrap().as_u64().ok_or_else(|| {
                        DataResourceService::invalid(
                            "structured_values_invalid",
                            &format!("字段 {name} 必须是非负整数"),
                        )
                    })?)
                    .map_err(|_| {
                        DataResourceService::invalid(
                            "structured_values_invalid",
                            &format!("字段 {name} 超出 u32 范围"),
                        )
                    })?,
                )
            };
            Ok((DataType::UInt32, Arc::new(UInt32Array::from(vec![item]))))
        }
        DataTypeContract::Primitive { name: kind } | DataTypeContract::Semantic { name: kind }
            if kind == "i64" || kind.contains("int") =>
        {
            let item = if null {
                None
            } else {
                Some(value.unwrap().as_i64().ok_or_else(|| {
                    DataResourceService::invalid(
                        "structured_values_invalid",
                        &format!("字段 {name} 必须是整数"),
                    )
                })?)
            };
            Ok((DataType::Int64, Arc::new(Int64Array::from(vec![item]))))
        }
        DataTypeContract::Constructor {
            name: kind,
            element,
        } if kind == "list"
            && matches!(element.as_ref(),DataTypeContract::Primitive{name} if name=="f64") =>
        {
            let items = if null {
                None
            } else {
                Some(
                    value
                        .unwrap()
                        .as_array()
                        .ok_or_else(|| {
                            DataResourceService::invalid(
                                "structured_values_invalid",
                                &format!("字段 {name} 必须是数组"),
                            )
                        })?
                        .iter()
                        .map(|item| {
                            item.as_f64().ok_or_else(|| {
                                DataResourceService::invalid(
                                    "structured_values_invalid",
                                    &format!("字段 {name} 包含非数值"),
                                )
                            })
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                )
            };
            let array = ListArray::from_iter_primitive::<Float64Type, _, _>(vec![
                items.map(|row| row.into_iter().map(Some).collect::<Vec<_>>()),
            ]);
            Ok((array.data_type().clone(), Arc::new(array)))
        }
        _ => Err(DataResourceService::invalid(
            "structured_values_invalid",
            &format!("字段 {name} 的 dtype 暂不支持"),
        )),
    }
}
fn dtype_label(dtype: &DataTypeContract) -> String {
    match dtype {
        DataTypeContract::Primitive { name } | DataTypeContract::Semantic { name } => name.clone(),
        DataTypeContract::Constructor { name, element } => {
            format!("{}<{}>", name, dtype_label(element))
        }
    }
}
