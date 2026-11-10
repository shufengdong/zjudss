//! 统一上传批次状态机：接收、解析、候选识别、用途选择、过期和恢复。
//!
//! 候选确认前不会修改模型草稿或资源库；解析只调用 ingestion 和 adapter 的受控边界。

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::adapters::SimEngine;
use crate::auth::CurrentUser;
use crate::catalog::DataFrameContractKind;
use crate::db::repositories::{
    NewUploadFile, RepoError, UploadBatchRecordSet, UploadConfirmationRepo, UploadRepo,
};
use crate::engine_resources::EngineResourceSnapshot;
use crate::ingestion::ArchiveLimits;

use super::resource_alias::normalize_alias;
use super::upload_candidate_policy::{
    CandidatePolicyError, selected_for_storage, validate_mutable_state, validate_selected_alias,
};
use super::upload_processing::UploadProcessor;
use super::{ArtifactError, ArtifactService, now_ms};

#[derive(Debug)]
pub struct UploadFile {
    pub client_file_id: String,
    pub file_name: String,
    pub relative_path: Option<String>,
    pub media_type: Option<String>,
    pub staged: crate::storage::StagedArtifact,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UploadBatchStatus {
    Processing,
    AwaitingParseSettings,
    AwaitingPurpose,
    ReadyToConfirm,
    PartiallyConfirmed,
    Confirmed,
    Failed,
    Discarded,
    Expired,
}

impl UploadBatchStatus {
    fn parse(value: &str) -> Self {
        match value {
            "awaiting_parse_settings" => Self::AwaitingParseSettings,
            "awaiting_purpose" => Self::AwaitingPurpose,
            "ready_to_confirm" => Self::ReadyToConfirm,
            "partially_confirmed" => Self::PartiallyConfirmed,
            "confirmed" => Self::Confirmed,
            "failed" => Self::Failed,
            "discarded" => Self::Discarded,
            "expired" => Self::Expired,
            _ => Self::Processing,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildPublishStatus {
    NotApplicable,
    Ready,
    Published,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidatePurpose {
    ModelTable,
    DataResource,
    Skip,
}
impl CandidatePurpose {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::ModelTable => "model_table",
            Self::DataResource => "data_resource",
            Self::Skip => "skip",
        }
    }
    fn parse(value: &str) -> Option<Self> {
        match value {
            "model_table" => Some(Self::ModelTable),
            "data_resource" => Some(Self::DataResource),
            "skip" => Some(Self::Skip),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UploadSourceKind {
    Upload,
    Case,
}
impl UploadSourceKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Case => "case",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ParseSettings {
    pub delimiter: Option<u8>,
    pub encoding: String,
    pub header_row: u32,
    pub decimal: String,
    pub thousands: Option<String>,
    pub datetime_format: Option<String>,
    pub timezone: Option<String>,
    pub excel_range: Option<String>,
}
impl Default for ParseSettings {
    fn default() -> Self {
        Self {
            delimiter: None,
            encoding: "utf-8".into(),
            header_row: 0,
            decimal: ".".into(),
            thousands: None,
            datetime_format: None,
            timezone: None,
            excel_range: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CandidateDecision {
    pub candidate_id: String,
    pub purpose: CandidatePurpose,
    pub selected: bool,
    pub table_name: Option<String>,
    pub display_name: Option<String>,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadCandidateView {
    pub id: String,
    pub sheet_name: Option<String>,
    pub display_name: String,
    pub proposed_alias: Option<String>,
    pub purpose: Option<CandidatePurpose>,
    pub inferred_schema_ref: Option<String>,
    pub schema: Option<serde_json::Value>,
    pub row_count: Option<i64>,
    pub evidence: serde_json::Value,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadBatchView {
    pub id: String,
    pub session_id: String,
    pub client_batch_id: String,
    pub source_kind: String,
    pub replacement_resource_id: Option<String>,
    pub status: UploadBatchStatus,
    pub model_publish_status: ChildPublishStatus,
    pub resource_publish_status: ChildPublishStatus,
    pub parse_settings: serde_json::Value,
    pub diagnostics: serde_json::Value,
    pub reserved_bytes: i64,
    pub expires_at_ms: i64,
    pub revision: i64,
    pub candidates: Vec<UploadCandidateView>,
}

#[derive(Debug)]
pub enum UploadBatchError {
    NotFound,
    Invalid(String, String),
    Expired,
    IdempotencyConflict,
    Artifact(ArtifactError),
    Repository(RepoError),
    Io(std::io::Error),
}
impl UploadBatchError {
    pub fn code(&self) -> &str {
        match self {
            Self::NotFound => "not_found",
            Self::Invalid(code, _) => code,
            Self::Expired => "upload_batch_expired",
            Self::IdempotencyConflict => "idempotency_conflict",
            Self::Artifact(ArtifactError::QuotaExceeded)
            | Self::Repository(RepoError::QuotaExceeded) => "storage_quota_exceeded",
            Self::Artifact(_) | Self::Repository(_) | Self::Io(_) => "upload_failed",
        }
    }
}
impl std::fmt::Display for UploadBatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "资源不存在"),
            Self::Invalid(_, m) => write!(f, "{m}"),
            Self::Expired => write!(f, "上传批次已过期"),
            Self::IdempotencyConflict => write!(f, "clientBatchId 已用于不同内容"),
            Self::Artifact(e) => write!(f, "{e}"),
            Self::Repository(e) => write!(f, "{e}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}
impl std::error::Error for UploadBatchError {}
impl From<RepoError> for UploadBatchError {
    fn from(e: RepoError) -> Self {
        match e {
            RepoError::NotFound => Self::NotFound,
            RepoError::InvalidState(code) if code == "upload_state_invalid" => Self::Invalid(
                "upload_state_invalid".into(),
                "上传批次正在确认或当前状态不可修改".into(),
            ),
            other => Self::Repository(other),
        }
    }
}
impl From<ArtifactError> for UploadBatchError {
    fn from(e: ArtifactError) -> Self {
        Self::Artifact(e)
    }
}
impl From<std::io::Error> for UploadBatchError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e)
    }
}

#[derive(Clone)]
/// 拥有上传批次从接收、解析到待确认/终态生命周期的应用门面。
pub struct UploadBatchService {
    repo: UploadRepo,
    confirmation_repo: UploadConfirmationRepo,
    processor: UploadProcessor,
    snapshot: Arc<EngineResourceSnapshot>,
    data_dir: PathBuf,
    ttl_ms: i64,
}

pub(crate) struct UploadBatchServiceDependencies {
    pub repo: UploadRepo,
    pub confirmation_repo: UploadConfirmationRepo,
    pub artifacts: ArtifactService,
    pub engine: Arc<dyn SimEngine>,
    pub snapshot: Arc<EngineResourceSnapshot>,
    pub data_dir: PathBuf,
    pub ttl_hours: u64,
    pub archive_limits: ArchiveLimits,
}

impl UploadBatchService {
    pub(crate) fn new(dependencies: UploadBatchServiceDependencies) -> Self {
        let UploadBatchServiceDependencies {
            repo,
            confirmation_repo,
            artifacts,
            engine,
            snapshot,
            data_dir,
            ttl_hours,
            archive_limits,
        } = dependencies;
        let processor = UploadProcessor::new(
            repo.clone(),
            artifacts,
            engine,
            snapshot.clone(),
            data_dir.clone(),
            archive_limits,
        );
        Self {
            repo,
            confirmation_repo,
            processor,
            snapshot,
            data_dir,
            ttl_ms: i64::try_from(ttl_hours.saturating_mul(3_600_000)).unwrap_or(i64::MAX),
        }
    }
    pub(crate) fn artifact_store(&self) -> &crate::storage::ArtifactStore {
        self.processor.artifact_store()
    }

    pub async fn accept(
        &self,
        user: &CurrentUser,
        session_id: i32,
        client_batch_id: String,
        source_kind: UploadSourceKind,
        files: Vec<UploadFile>,
    ) -> Result<UploadBatchView, UploadBatchError> {
        self.accept_with_target(user, session_id, client_batch_id, source_kind, None, files)
            .await
    }

    pub async fn accept_replacement(
        &self,
        user: &CurrentUser,
        session_id: i32,
        client_batch_id: String,
        resource_id: String,
        files: Vec<UploadFile>,
    ) -> Result<UploadBatchView, UploadBatchError> {
        self.accept_with_target(
            user,
            session_id,
            client_batch_id,
            UploadSourceKind::Upload,
            Some(resource_id),
            files,
        )
        .await
    }

    async fn accept_with_target(
        &self,
        user: &CurrentUser,
        session_id: i32,
        client_batch_id: String,
        source_kind: UploadSourceKind,
        replacement_resource_id: Option<String>,
        files: Vec<UploadFile>,
    ) -> Result<UploadBatchView, UploadBatchError> {
        if client_batch_id.trim().is_empty() || files.is_empty() {
            return Err(UploadBatchError::Invalid(
                "upload_invalid".into(),
                "clientBatchId 和文件不能为空".into(),
            ));
        }
        let mut client_file_ids = BTreeSet::new();
        if files.iter().any(|file| {
            file.client_file_id.trim().is_empty()
                || !client_file_ids.insert(file.client_file_id.as_str())
        }) {
            return Err(UploadBatchError::Invalid(
                "upload_invalid".into(),
                "clientFileId 必须非空且唯一".into(),
            ));
        }
        if let Some(existing) = self
            .repo
            .find_by_client_id(&user.id, session_id, &client_batch_id)
            .await?
        {
            let incoming = source_fingerprint(files.iter().map(|file| {
                (
                    &file.client_file_id,
                    &file.file_name,
                    file.relative_path.as_deref(),
                    file.staged.sha256.as_str(),
                )
            }));
            let stored = source_fingerprint(existing.files.iter().map(|file| {
                (
                    &file.client_file_id,
                    &file.file_name,
                    file.relative_path.as_deref(),
                    file.sha256.as_str(),
                )
            }));
            return if incoming == stored
                && existing.batch.replacement_resource_id == replacement_resource_id
            {
                Ok(view(existing))
            } else {
                Err(UploadBatchError::IdempotencyConflict)
            };
        }
        let total = files
            .iter()
            .try_fold(0_i64, |sum, file| {
                i64::try_from(file.staged.size_bytes)
                    .ok()
                    .and_then(|size| sum.checked_add(size))
            })
            .ok_or_else(|| {
                UploadBatchError::Invalid(
                    "upload_batch_too_large".into(),
                    "上传批次大小溢出".into(),
                )
            })?;
        let now = now_ms();
        let expected_replacement_resource_id = replacement_resource_id.clone();
        let (batch, created) = self
            .repo
            .create_reserved(
                &user.id,
                user.storage_scope_id,
                session_id,
                client_batch_id,
                source_kind.as_str().into(),
                replacement_resource_id,
                total,
                now.saturating_add(self.ttl_ms),
                now,
            )
            .await?;
        if !created {
            let existing = self
                .repo
                .find_by_client_id(&user.id, session_id, &batch.client_batch_id)
                .await?
                .ok_or(UploadBatchError::IdempotencyConflict)?;
            let incoming = source_fingerprint(files.iter().map(|file| {
                (
                    &file.client_file_id,
                    &file.file_name,
                    file.relative_path.as_deref(),
                    file.staged.sha256.as_str(),
                )
            }));
            let stored = source_fingerprint(existing.files.iter().map(|file| {
                (
                    &file.client_file_id,
                    &file.file_name,
                    file.relative_path.as_deref(),
                    file.sha256.as_str(),
                )
            }));
            return if !existing.files.is_empty()
                && incoming == stored
                && existing.batch.replacement_resource_id == expected_replacement_resource_id
            {
                Ok(view(existing))
            } else {
                Err(UploadBatchError::IdempotencyConflict)
            };
        }
        let root = self
            .data_dir
            .join("upload-batches")
            .join(batch.id.to_string());
        let mut rows = Vec::new();
        for upload in files {
            let file_id = stable_id(
                "file",
                &format!(
                    "{}:{}:{}",
                    batch.id, upload.client_file_id, upload.staged.sha256
                ),
            );
            let destination = root.join(&file_id);
            let size = match i64::try_from(upload.staged.size_bytes) {
                Ok(value) => value,
                Err(_) => {
                    let _ = self
                        .repo
                        .settle(&user.id, batch.id, "failed", now_ms())
                        .await;
                    let _ = std::fs::remove_dir_all(&root);
                    return Err(UploadBatchError::Invalid(
                        "upload_file_too_large".into(),
                        "文件过大".into(),
                    ));
                }
            };
            let sha = upload.staged.sha256.clone();
            if let Err(error) = upload.staged.persist(&destination) {
                let _ = self
                    .repo
                    .settle(&user.id, batch.id, "failed", now_ms())
                    .await;
                let _ = std::fs::remove_dir_all(&root);
                return Err(UploadBatchError::Invalid(
                    error.code().into(),
                    error.to_string(),
                ));
            }
            rows.push(NewUploadFile {
                id: file_id,
                batch_id: batch.id,
                client_file_id: upload.client_file_id,
                file_name: upload.file_name,
                relative_path: upload.relative_path,
                media_type: upload.media_type,
                staged_path: destination
                    .strip_prefix(&self.data_dir)
                    .unwrap()
                    .to_string_lossy()
                    .to_string(),
                size_bytes: size,
                sha256: sha,
                source_artifact_id: None,
                created_at_ms: now,
            });
        }
        if let Err(error) = self.repo.add_files(rows).await {
            let _ = self
                .repo
                .settle(&user.id, batch.id, "failed", now_ms())
                .await;
            let _ = std::fs::remove_dir_all(root);
            return Err(error.into());
        }
        self.process(user, batch.id, ParseSettings::default()).await
    }

    pub async fn get(
        &self,
        user: &CurrentUser,
        id: i32,
    ) -> Result<UploadBatchView, UploadBatchError> {
        let record = self.repo.find(&user.id, id).await?;
        if record.batch.expires_at_ms <= now_ms()
            && !matches!(
                record.batch.status.as_str(),
                "discarded" | "expired" | "confirmed" | "failed"
            )
        {
            self.settle(user, id, "expired").await?;
            return Ok(view(self.repo.find(&user.id, id).await?));
        }
        let mut result = view(record.clone());
        // Older batches retained only the stable code. Enrich their read view without
        // changing the frozen confirmation receipt or publishing any resources.
        if record.batch.resource_publish_status == "failed"
            && record
                .batch
                .resource_diagnostics_json
                .contains("resource_alias_conflict")
            && !record.batch.resource_diagnostics_json.contains("别名：")
        {
            let existing = self
                .confirmation_repo
                .list_current_for_user(&user.id)
                .await?;
            let conflicts = record
                .candidates
                .iter()
                .filter(|candidate| {
                    candidate.selected && candidate.purpose.as_deref() == Some("data_resource")
                })
                .filter_map(|candidate| {
                    let alias = candidate.normalized_alias.as_deref()?;
                    existing
                        .iter()
                        .find(|resource| {
                            resource.session_id == record.batch.session_id
                                && resource.normalized_alias == alias
                        })
                        .map(|_| format!("{}（别名：{}）", candidate.display_name, alias))
                })
                .collect::<Vec<_>>();
            if !conflicts.is_empty() {
                let message = format!(
                    "当前仍重名的数据资源：{}。请修改别名后重试数据发布。",
                    conflicts.join("、")
                );
                let errors = if result.diagnostics.is_array() {
                    result.diagnostics.as_array_mut()
                } else {
                    result
                        .diagnostics
                        .get_mut("errors")
                        .and_then(serde_json::Value::as_array_mut)
                };
                if let Some(errors) = errors {
                    for error in errors {
                        if error["code"] == "resource_alias_conflict" {
                            error["message"] = serde_json::json!(message);
                        }
                    }
                }
            }
        }
        Ok(result)
    }
    pub async fn count_by_client_id(
        &self,
        user: &CurrentUser,
        session_id: i32,
        client: &str,
    ) -> Result<i64, UploadBatchError> {
        Ok(self
            .repo
            .count_by_client_id(&user.id, session_id, client)
            .await?)
    }
    pub async fn replacement_session(
        &self,
        user: &CurrentUser,
        resource_id: &str,
    ) -> Result<i32, UploadBatchError> {
        Ok(self
            .repo
            .session_for_resource(&user.id, resource_id)
            .await?)
    }
    pub async fn recover_startup(&self) -> Result<(), UploadBatchError> {
        self.repo.release_abandoned_confirmations(now_ms()).await?;
        for item in self.repo.recoverable().await? {
            let user = CurrentUser {
                id: crate::auth::UserId(item.user_id),
                name: item.user_name,
                storage_scope_id: item.storage_scope_id,
                auth_session_hash: String::new(),
            };
            let mut record = self.repo.find(&user.id, item.batch_id).await?;
            if record.batch.confirmation_started {
                let cleanup = self
                    .confirmation_repo
                    .finish_batch(&user.id, item.batch_id, now_ms())
                    .await?;
                for path in cleanup.staged_paths {
                    let _ = std::fs::remove_file(self.data_dir.join(path));
                }
                record = self.repo.find(&user.id, item.batch_id).await?;
                if record.batch.status == "confirmed" {
                    continue;
                }
            }
            if record.batch.expires_at_ms <= now_ms() {
                self.settle(&user, item.batch_id, "expired").await?;
                continue;
            }
            let missing = record.files.is_empty()
                || record
                    .files
                    .iter()
                    .any(|file| !self.data_dir.join(&file.staged_path).is_file());
            if missing {
                self.repo.replace_candidates(item.batch_id,Vec::new(),"processing",record.batch.parse_settings_json.clone(),serde_json::json!([{"code":"upload_staging_missing","message":"上传暂存文件已丢失"}]).to_string(),now_ms()).await?;
                self.processor
                    .release_previous(user.storage_scope_id, &record)
                    .await;
                self.settle(&user, item.batch_id, "failed").await?;
                continue;
            }
            if record.batch.status == "processing" {
                let settings =
                    serde_json::from_str(&record.batch.parse_settings_json).unwrap_or_default();
                let _ = self.process(&user, item.batch_id, settings).await?;
            }
        }
        Ok(())
    }

    pub async fn update_parse_settings(
        &self,
        user: &CurrentUser,
        id: i32,
        settings: ParseSettings,
    ) -> Result<UploadBatchView, UploadBatchError> {
        let batch = self.repo.find(&user.id, id).await?;
        if batch.batch.expires_at_ms <= now_ms() {
            self.settle(user, id, "expired").await?;
            return Err(UploadBatchError::Expired);
        }
        self.ensure_mutable(&batch)?;
        self.process(user, id, settings).await
    }

    pub async fn update_candidates(
        &self,
        user: &CurrentUser,
        id: i32,
        decisions: Vec<CandidateDecision>,
    ) -> Result<UploadBatchView, UploadBatchError> {
        self.update_candidates_inner(user, id, decisions, None, None)
            .await
    }

    pub(crate) async fn update_candidates_with_alias_intent(
        &self,
        user: &CurrentUser,
        id: i32,
        decisions: Vec<CandidateDecision>,
        explicit_aliases: BTreeSet<String>,
    ) -> Result<UploadBatchView, UploadBatchError> {
        self.update_candidates_inner(user, id, decisions, None, Some(explicit_aliases))
            .await
    }

    pub(crate) async fn confirm_candidates(
        &self,
        user: &CurrentUser,
        id: i32,
        decisions: Vec<CandidateDecision>,
        operation_id: &str,
        payload_hash: &str,
        explicit_aliases: BTreeSet<String>,
    ) -> Result<UploadBatchView, UploadBatchError> {
        self.update_candidates_inner(
            user,
            id,
            decisions,
            Some((operation_id.to_string(), payload_hash.to_string())),
            Some(explicit_aliases),
        )
        .await
    }

    async fn update_candidates_inner(
        &self,
        user: &CurrentUser,
        id: i32,
        decisions: Vec<CandidateDecision>,
        confirmation: Option<(String, String)>,
        explicit_aliases: Option<BTreeSet<String>>,
    ) -> Result<UploadBatchView, UploadBatchError> {
        let batch = self.repo.find(&user.id, id).await?;
        if batch.batch.expires_at_ms <= now_ms() {
            self.settle(user, id, "expired").await?;
            return Err(UploadBatchError::Expired);
        }
        self.ensure_mutable(&batch)?;
        let known = batch
            .candidates
            .iter()
            .map(|candidate| (candidate.id.as_str(), candidate))
            .collect::<BTreeMap<_, _>>();
        let decided = decisions
            .iter()
            .map(|decision| decision.candidate_id.as_str())
            .collect::<BTreeSet<_>>();
        if decisions.is_empty()
            || decisions.len() != known.len()
            || decided.len() != known.len()
            || decided.iter().any(|id| !known.contains_key(id))
        {
            return Err(UploadBatchError::Invalid(
                "upload_candidate_invalid".into(),
                "必须为每个候选提交一次用途决定".into(),
            ));
        }
        for decision in &decisions {
            let existing = known[decision.candidate_id.as_str()];
            let existing_purpose = existing
                .purpose
                .as_deref()
                .and_then(CandidatePurpose::parse);
            let model_side_published = batch.batch.model_publish_status == "published"
                && (existing_purpose == Some(CandidatePurpose::ModelTable)
                    || decision.purpose == CandidatePurpose::ModelTable);
            let resource_side_published = batch.batch.resource_publish_status == "published"
                && (existing_purpose == Some(CandidatePurpose::DataResource)
                    || decision.purpose == CandidatePurpose::DataResource);
            let published_child = model_side_published || resource_side_published;
            if published_child {
                let requested_alias = decision
                    .alias
                    .as_deref()
                    .or(decision.table_name.as_deref())
                    .or(existing.proposed_alias.as_deref())
                    .map(normalize_alias);
                let existing_alias = existing.proposed_alias.as_deref().map(normalize_alias);
                let display_changed = decision
                    .display_name
                    .as_deref()
                    .is_some_and(|display| display != existing.display_name);
                if decision.purpose != existing_purpose.unwrap()
                    || decision.selected != existing.selected
                    || requested_alias != existing_alias
                    || display_changed
                {
                    return Err(UploadBatchError::Invalid(
                        "upload_candidate_invalid".into(),
                        "已发布子事务的候选决定不可修改".into(),
                    ));
                }
            }
            if !decision.selected || decision.purpose == CandidatePurpose::Skip {
                continue;
            }
            let contract_kind = existing
                .inferred_schema_ref
                .as_deref()
                .and_then(|schema_ref| self.snapshot.schemas.resolve(schema_ref).ok())
                .map(|contract| &contract.kind);
            let purpose_matches_contract = matches!(
                (decision.purpose, contract_kind),
                (
                    CandidatePurpose::ModelTable,
                    Some(DataFrameContractKind::Model)
                ) | (CandidatePurpose::ModelTable, None)
                    | (
                        CandidatePurpose::DataResource,
                        Some(DataFrameContractKind::Struct)
                    )
                    | (
                        CandidatePurpose::DataResource,
                        Some(DataFrameContractKind::Data)
                    )
                    | (
                        CandidatePurpose::DataResource,
                        Some(DataFrameContractKind::Result)
                    )
                    | (CandidatePurpose::DataResource, None)
            );
            if !purpose_matches_contract {
                return Err(UploadBatchError::Invalid(
                    "upload_candidate_invalid".into(),
                    "候选用途与当前引擎 schema 的 input kind 不一致".into(),
                ));
            }
        }
        let values = decisions
            .into_iter()
            .map(|d| {
                let existing = known[d.candidate_id.as_str()];
                let requested_alias = d.alias.or(d.table_name);
                let alias_explicit = existing.alias_explicit
                    || explicit_aliases.as_ref().map_or_else(
                        || {
                            requested_alias.as_deref().is_some_and(|alias| {
                                existing.proposed_alias.as_deref().is_none_or(|suggested| {
                                    normalize_alias(suggested) != normalize_alias(alias)
                                })
                            })
                        },
                        |aliases| aliases.contains(&d.candidate_id),
                    );
                let alias = requested_alias.or_else(|| existing.proposed_alias.clone());
                let selected = selected_for_storage(d.selected, d.purpose);
                if validate_selected_alias(d.selected, d.purpose, alias.as_deref()).is_err() {
                    return Err(UploadBatchError::Invalid(
                        "upload_candidate_invalid".into(),
                        "已选择候选必须提供表名或 alias".into(),
                    ));
                }
                let normalized = alias.as_deref().map(normalize_alias);
                Ok((
                    d.candidate_id,
                    d.purpose.as_str().into(),
                    selected,
                    d.display_name,
                    alias,
                    normalized,
                    alias_explicit,
                ))
            })
            .collect::<Result<Vec<_>, UploadBatchError>>()?;
        self.repo
            .update_decisions(id, values, "ready_to_confirm", confirmation, now_ms())
            .await?;
        self.get(user, id).await
    }

    pub async fn discard(&self, user: &CurrentUser, id: i32) -> Result<(), UploadBatchError> {
        self.settle(user, id, "discarded").await
    }
    pub async fn expire_before(
        &self,
        user: &CurrentUser,
        before: i64,
    ) -> Result<(), UploadBatchError> {
        for id in self.repo.ids_expiring_before(&user.id, before).await? {
            self.settle(user, id, "expired").await?;
        }
        Ok(())
    }

    async fn settle(
        &self,
        user: &CurrentUser,
        id: i32,
        status: &str,
    ) -> Result<(), UploadBatchError> {
        let cleanup = self.repo.settle(&user.id, id, status, now_ms()).await?;
        for path in cleanup.staged_paths {
            let _ = std::fs::remove_file(self.data_dir.join(path));
        }
        Ok(())
    }

    fn ensure_mutable(&self, batch: &UploadBatchRecordSet) -> Result<(), UploadBatchError> {
        if batch.batch.expires_at_ms <= now_ms() || batch.batch.status == "expired" {
            Err(UploadBatchError::Expired)
        } else {
            match validate_mutable_state(&batch.batch.status, batch.batch.confirmation_in_progress)
            {
                Ok(()) => Ok(()),
                Err(CandidatePolicyError::ConfirmationInProgress) => {
                    Err(UploadBatchError::Invalid(
                        "upload_state_invalid".into(),
                        "上传确认正在执行，不能修改批次".into(),
                    ))
                }
                Err(_) => Err(UploadBatchError::Invalid(
                    "upload_state_invalid".into(),
                    "上传批次不可再修改".into(),
                )),
            }
        }
    }

    async fn process(
        &self,
        user: &CurrentUser,
        id: i32,
        settings: ParseSettings,
    ) -> Result<UploadBatchView, UploadBatchError> {
        let record = self.repo.find(&user.id, id).await?;
        self.ensure_mutable(&record)?;
        self.processor.validate_settings(&settings)?;
        let case_switch_state = async {
            self.confirmation_repo
                .case_switch_state(&user.id, record.batch.session_id)
                .await
                .map_err(UploadBatchError::from)
        };
        let result = self
            .processor
            .process(user, &record, &settings, case_switch_state)
            .await;
        if let Err(error) = result {
            let recoverable_parse = matches!(&error,UploadBatchError::Invalid(code,_) if code.starts_with("tabular_")||code=="upload_parse_settings_invalid");
            self.repo
                .replace_candidates(
                    id,
                    Vec::new(),
                    if recoverable_parse {
                        "awaiting_parse_settings"
                    } else {
                        "processing"
                    },
                    serde_json::to_string(&settings).unwrap(),
                    self.processor
                        .diagnostics(&record, &settings, &error)
                        .to_string(),
                    now_ms(),
                )
                .await?;
            self.processor
                .release_previous(user.storage_scope_id, &record)
                .await;
            if !recoverable_parse {
                self.settle(user, id, "failed").await?;
            }
        }
        self.get(user, id).await
    }
}

fn view(record: UploadBatchRecordSet) -> UploadBatchView {
    let mut diagnostics: serde_json::Value =
        serde_json::from_str(&record.batch.diagnostics_json).unwrap_or_default();
    let child_errors = [
        &record.batch.model_diagnostics_json,
        &record.batch.resource_diagnostics_json,
    ]
    .into_iter()
    .flat_map(|raw| serde_json::from_str::<Vec<serde_json::Value>>(raw).unwrap_or_default())
    .collect::<Vec<_>>();
    let mut unique_errors = Vec::new();
    for error in child_errors {
        if !unique_errors.contains(&error) {
            unique_errors.push(error);
        }
    }
    let child_errors = unique_errors;
    if !child_errors.is_empty() {
        if diagnostics.is_array() {
            diagnostics.as_array_mut().unwrap().extend(child_errors);
        } else {
            if !diagnostics.is_object() {
                diagnostics = serde_json::json!({});
            }
            let errors = diagnostics
                .as_object_mut()
                .unwrap()
                .entry("errors")
                .or_insert_with(|| serde_json::json!([]));
            if let Some(errors) = errors.as_array_mut() {
                errors.extend(child_errors);
            }
        }
    }
    UploadBatchView {
        id: record.batch.id.to_string(),
        session_id: record.batch.session_id.to_string(),
        client_batch_id: record.batch.client_batch_id,
        source_kind: record.batch.source_kind,
        replacement_resource_id: record.batch.replacement_resource_id,
        status: UploadBatchStatus::parse(&record.batch.status),
        model_publish_status: parse_child(&record.batch.model_publish_status),
        resource_publish_status: parse_child(&record.batch.resource_publish_status),
        parse_settings: serde_json::from_str(&record.batch.parse_settings_json).unwrap_or_default(),
        diagnostics,
        reserved_bytes: record.batch.reserved_bytes,
        expires_at_ms: record.batch.expires_at_ms,
        revision: record.batch.revision,
        candidates: record
            .candidates
            .into_iter()
            .map(|c| UploadCandidateView {
                id: c.id,
                sheet_name: c.sheet_name,
                display_name: c.display_name,
                proposed_alias: c.proposed_alias,
                purpose: c.purpose.as_deref().and_then(CandidatePurpose::parse),
                inferred_schema_ref: c.inferred_schema_ref,
                schema: c.schema_json.and_then(|v| serde_json::from_str(&v).ok()),
                row_count: c.row_count,
                evidence: serde_json::from_str(&c.evidence_json).unwrap_or_default(),
                selected: c.selected,
            })
            .collect(),
    }
}
fn parse_child(value: &str) -> ChildPublishStatus {
    match value {
        "ready" => ChildPublishStatus::Ready,
        "published" => ChildPublishStatus::Published,
        "failed" => ChildPublishStatus::Failed,
        _ => ChildPublishStatus::NotApplicable,
    }
}
pub(super) fn stable_id(prefix: &str, value: &str) -> String {
    format!(
        "{prefix}_{}",
        &hex::encode(Sha256::digest(value.as_bytes()))[..24]
    )
}
pub(super) fn source_fingerprint<'a>(
    files: impl IntoIterator<Item = (&'a String, &'a String, Option<&'a str>, &'a str)>,
) -> String {
    let mut rows = files
        .into_iter()
        .map(|(id, name, path, sha)| format!("{id}\0{name}\0{}\0{sha}", path.unwrap_or_default()))
        .collect::<Vec<_>>();
    rows.sort();
    hex::encode(Sha256::digest(rows.join("\n").as_bytes()))
}
