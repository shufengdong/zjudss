use std::collections::BTreeMap;
use std::fs::File;
use std::sync::Arc;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::auth::{CurrentUser, UserId};
use crate::catalog::DataFrameContractKind;
use crate::db::repositories::{
    DataResourceRepo, PromoteDatasetInput, RepoError, ResourcePageRow, RetryStructOverlayRequest,
    WorkflowStructOverlayMaterial,
};
use crate::engine_resources::EngineResourceSnapshot;

use super::dataframe_reader::{DataFrameReadError, DataFrameReadSource, ParquetDataFrameReader};
use super::resource_alias::{
    AliasError, AliasPurpose, ResourceAliasPolicy, default_alias, normalize_alias,
};
use super::{ArtifactError, ArtifactService, CandidatePurpose, UploadBatchError, now_ms};

#[path = "study_struct_overlays.rs"]
pub(super) mod study_struct_overlays;
use self::study_struct_overlays::materialize_struct_overlay;
#[path = "data_resource_deletion.rs"]
mod data_resource_deletion;
#[path = "data_resource_parquet.rs"]
mod data_resource_parquet;
#[path = "data_resource_replacement.rs"]
mod data_resource_replacement;

pub use self::data_resource_deletion::{DataResourceDeletionPreview, DeletedDataResource};
use self::data_resource_parquet::struct_parquet;
pub use self::data_resource_replacement::StructuredReplacementPreview;

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
