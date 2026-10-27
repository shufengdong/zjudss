use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::adapters::{ImportSpec, MODEL_TABLE_OUTPUTS, PARA_RESOURCE_OUTPUT, SimEngine};
use crate::auth::CurrentUser;
use crate::catalog::DataFrameContractKind;
use crate::db::repositories::{
    DataResourceRepo, NewUploadCandidate, NewUploadFile, RepoError, UploadBatchRecordSet,
    UploadRepo,
};
use crate::engine_resources::{EngineResourceSnapshot, FacadeInputKind};
use crate::ingestion::{
    ArchiveExtractor, ArchiveLimits, normalize_csv_with_settings, normalize_excel_to_parquet,
};
use crate::storage::{ParquetInspector, StageLimits};

use super::{ArtifactError, ArtifactMetadata, ArtifactService, now_ms};

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
    fn as_str(self) -> &'static str {
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

struct ParsedTabularCandidate {
    file_id: String,
    sheet: Option<String>,
    display: String,
    path: PathBuf,
    metadata: crate::storage::ParquetMetadata,
    alias: Option<String>,
    purpose: Option<CandidatePurpose>,
    schema_ref: Option<String>,
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
pub struct UploadBatchService {
    repo: UploadRepo,
    confirmation_repo: DataResourceRepo,
    artifacts: ArtifactService,
    engine: Arc<dyn SimEngine>,
    snapshot: Arc<EngineResourceSnapshot>,
    data_dir: PathBuf,
    ttl_ms: i64,
    archive_limits: ArchiveLimits,
}

pub(crate) struct UploadBatchServiceDependencies {
    pub repo: UploadRepo,
    pub confirmation_repo: DataResourceRepo,
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
        Self {
            repo,
            confirmation_repo,
            artifacts,
            engine,
            snapshot,
            data_dir,
            ttl_ms: i64::try_from(ttl_hours.saturating_mul(3_600_000)).unwrap_or(i64::MAX),
            archive_limits,
        }
    }
    pub(crate) fn artifact_store(&self) -> &crate::storage::ArtifactStore {
        self.artifacts.store()
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
        Ok(view(record))
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
                self.release_previous(user.storage_scope_id, &record).await;
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
                let selected = d.selected && d.purpose != CandidatePurpose::Skip;
                if selected && alias.as_deref().is_none_or(|value| value.trim().is_empty()) {
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
        } else if matches!(
            batch.batch.status.as_str(),
            "failed" | "discarded" | "confirmed"
        ) {
            Err(UploadBatchError::Invalid(
                "upload_state_invalid".into(),
                "上传批次不可再修改".into(),
            ))
        } else {
            Ok(())
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
        if !settings.encoding.eq_ignore_ascii_case("utf-8")
            || !matches!(settings.decimal.as_str(), "." | ",")
            || settings
                .delimiter
                .is_some_and(|value| !matches!(value, b',' | b';' | b'\t' | b'|'))
        {
            return Err(UploadBatchError::Invalid(
                "upload_parse_settings_invalid".into(),
                "解析设置不受支持".into(),
            ));
        }
        let all_dss = record
            .files
            .iter()
            .all(|f| extension(&f.file_name) == "dss");
        let result = if record.files.len() == 1 && extension(&record.files[0].file_name) == "zip" {
            self.process_archive(user, &record, &settings).await
        } else if all_dss || record.batch.source_kind == "case" {
            self.process_dss(user, &record, &settings).await
        } else {
            self.process_tabular(user, &record, &settings).await
        };
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
                    batch_diagnostics(&record, &settings, Some(&error)).to_string(),
                    now_ms(),
                )
                .await?;
            self.release_previous(user.storage_scope_id, &record).await;
            if !recoverable_parse {
                self.settle(user, id, "failed").await?;
            }
        }
        self.get(user, id).await
    }

    async fn process_dss(
        &self,
        user: &CurrentUser,
        record: &UploadBatchRecordSet,
        settings: &ParseSettings,
    ) -> Result<(), UploadBatchError> {
        let input = self.materialize_tree(record)?;
        let file_id = record
            .files
            .iter()
            .find(|file| extension(&file.file_name) == "dss")
            .map(|file| file.id.as_str())
            .unwrap_or(&record.files[0].id);
        self.process_dss_tree(user, record, file_id, input, settings)
            .await
    }

    async fn process_archive(
        &self,
        user: &CurrentUser,
        record: &UploadBatchRecordSet,
        settings: &ParseSettings,
    ) -> Result<(), UploadBatchError> {
        let file = &record.files[0];
        let source = self.data_dir.join(&file.staged_path);
        let destination = self
            .data_dir
            .join("upload-work")
            .join(record.batch.id.to_string())
            .join("archive");
        let tree = ArchiveExtractor::extract(&source, &destination, self.archive_limits).map_err(
            |error| UploadBatchError::Invalid("archive_limits_exceeded".into(), error.to_string()),
        )?;
        if tree.files.is_empty()
            || !tree
                .files
                .values()
                .all(|entry| extension(&entry.relative_path) == "dss")
        {
            return Err(UploadBatchError::Invalid(
                "upload_format_unsupported".into(),
                "归档必须包含一个 DSS 文件树".into(),
            ));
        }
        self.process_dss_tree(user, record, &file.id, tree.root, settings)
            .await
    }

    async fn process_dss_tree(
        &self,
        user: &CurrentUser,
        record: &UploadBatchRecordSet,
        file_id: &str,
        input: PathBuf,
        settings: &ParseSettings,
    ) -> Result<(), UploadBatchError> {
        let inspection = self
            .engine
            .inspect_model_import(crate::adapters::ImportInspectSpec {
                input_tree: input.clone(),
                timeout: std::time::Duration::ZERO,
            })
            .await
            .map_err(|e| UploadBatchError::Invalid(e.code.clone(), e.to_string()))?;
        if inspection.entry_candidates.len() != 1 {
            return Err(UploadBatchError::Invalid(
                "model_import_resolution_required".into(),
                "DSS 入口不唯一".into(),
            ));
        }
        let results = self
            .data_dir
            .join("upload-work")
            .join(record.batch.id.to_string())
            .join("results");
        let imported = self
            .engine
            .import_model(ImportSpec {
                input_tree: input,
                entry_candidate_id: inspection.entry_candidates[0].id.clone(),
                result_dir: results,
                timeout: std::time::Duration::ZERO,
            })
            .await
            .map_err(|e| UploadBatchError::Invalid(e.code.clone(), e.to_string()))?;
        let mut candidates = Vec::new();
        let mut committed = Vec::new();
        for output in imported.dataframes {
            let is_para = output.name == PARA_RESOURCE_OUTPUT;
            if !is_para && !MODEL_TABLE_OUTPUTS.contains(&output.name.as_str()) {
                continue;
            }
            let artifact = match self.commit_path(user.storage_scope_id, &output.path).await {
                Ok(value) => value,
                Err(error) => {
                    self.release_ids(user.storage_scope_id, &committed).await;
                    return Err(error);
                }
            };
            committed.push(artifact.id);
            candidates.push(candidate(
                record.batch.id,
                file_id,
                None,
                &output.name,
                Some(&output.name),
                Some(if is_para {
                    CandidatePurpose::DataResource
                } else {
                    CandidatePurpose::ModelTable
                }),
                Some(format!("frames.{}", output.name)),
                artifact.id,
                &output.metadata,
                serde_json::json!({"source":"static_import","profile":imported.profile}),
            ));
        }
        if let Err(error) = self
            .repo
            .replace_candidates(
                record.batch.id,
                candidates,
                "ready_to_confirm",
                serde_json::to_string(settings).unwrap(),
                batch_diagnostics(record, settings, None).to_string(),
                now_ms(),
            )
            .await
        {
            self.release_ids(user.storage_scope_id, &committed).await;
            return Err(error.into());
        }
        self.release_previous(user.storage_scope_id, record).await;
        Ok(())
    }

    async fn process_tabular(
        &self,
        user: &CurrentUser,
        record: &UploadBatchRecordSet,
        settings: &ParseSettings,
    ) -> Result<(), UploadBatchError> {
        let mut parsed = Vec::new();
        let mut needs_settings = false;
        let mut needs_purpose = false;
        if record.files.iter().any(|file| {
            !matches!(
                extension(&file.file_name).as_str(),
                "csv" | "xls" | "xlsx" | "parquet"
            )
        }) {
            return Err(UploadBatchError::Invalid(
                "upload_format_unsupported".into(),
                "批次包含不支持的表格格式".into(),
            ));
        }
        for file in &record.files {
            let source = self.data_dir.join(&file.staged_path);
            let ext = extension(&file.file_name);
            let outputs = match ext.as_str() {
                "csv" => {
                    let bytes = std::fs::read(&source)?;
                    if settings.delimiter.is_none() && ambiguous_delimiter(&bytes) {
                        needs_settings = true;
                        continue;
                    }
                    let out = self
                        .data_dir
                        .join("upload-work")
                        .join(record.batch.id.to_string())
                        .join(format!("{}.parquet", file.id));
                    normalize_csv_with_settings(
                        &source,
                        &out,
                        settings.delimiter.unwrap_or(b','),
                        settings.header_row,
                    )
                    .map_err(|e| UploadBatchError::Invalid(e.code().into(), e.to_string()))?;
                    vec![(None, out)]
                }
                "xls" | "xlsx" => normalize_excel_to_parquet(
                    &source,
                    &self
                        .data_dir
                        .join("upload-work")
                        .join(record.batch.id.to_string())
                        .join(&file.id),
                )
                .map_err(|e| UploadBatchError::Invalid(e.code().into(), e.to_string()))?
                .into_iter()
                .map(|s| (Some(s.sheet_name), s.path))
                .collect(),
                "parquet" => vec![(None, source.clone())],
                _ => {
                    return Err(UploadBatchError::Invalid(
                        "upload_format_unsupported".into(),
                        format!("不支持的上传格式: {}", file.file_name),
                    ));
                }
            };
            for (sheet, path) in outputs {
                let metadata = ParquetInspector::metadata(&path)
                    .map_err(|e| UploadBatchError::Invalid(e.code().into(), e.to_string()))?;
                let inferred = self.infer_model(
                    &metadata
                        .columns
                        .iter()
                        .map(|c| c.name.clone())
                        .collect::<BTreeSet<_>>(),
                );
                let (alias, purpose, schema_ref) = match inferred {
                    Some((a, s)) => (Some(a), Some(CandidatePurpose::ModelTable), Some(s)),
                    None => {
                        needs_purpose = true;
                        (None, None, None)
                    }
                };
                let display = sheet.clone().unwrap_or_else(|| file.file_name.clone());
                parsed.push(ParsedTabularCandidate {
                    file_id: file.id.clone(),
                    sheet,
                    display,
                    path,
                    metadata,
                    alias,
                    purpose,
                    schema_ref,
                });
            }
        }
        let mut candidates = Vec::new();
        let mut committed = Vec::new();
        for parsed in parsed {
            let artifact = match self.commit_path(user.storage_scope_id, &parsed.path).await {
                Ok(value) => value,
                Err(error) => {
                    self.release_ids(user.storage_scope_id, &committed).await;
                    return Err(error);
                }
            };
            committed.push(artifact.id);
            candidates.push(candidate(record.batch.id,&parsed.file_id,parsed.sheet,&parsed.display,parsed.alias.as_deref(),parsed.purpose,parsed.schema_ref,artifact.id,&parsed.metadata,serde_json::json!({"source":"tabular","headerColumns":parsed.metadata.columns.iter().map(|c|&c.name).collect::<Vec<_>>() })));
        }
        let status = if needs_settings {
            "awaiting_parse_settings"
        } else if needs_purpose {
            "awaiting_purpose"
        } else {
            "ready_to_confirm"
        };
        if let Err(error) = self
            .repo
            .replace_candidates(
                record.batch.id,
                candidates,
                status,
                serde_json::to_string(settings).unwrap(),
                batch_diagnostics(record, settings, None).to_string(),
                now_ms(),
            )
            .await
        {
            self.release_ids(user.storage_scope_id, &committed).await;
            return Err(error.into());
        }
        self.release_previous(user.storage_scope_id, record).await;
        Ok(())
    }

    fn infer_model(&self, columns: &BTreeSet<String>) -> Option<(String, String)> {
        let mut matches = BTreeMap::<(usize, usize), (String, String)>::new();
        for facade in self.snapshot.facades.enabled() {
            for input in &facade.inputs {
                if input.kind != FacadeInputKind::Model {
                    continue;
                }
                if let Ok(schema) = self.snapshot.schemas.resolve(&input.schema_ref)
                    && schema.kind == DataFrameContractKind::Model
                {
                    let required = schema
                        .fields
                        .iter()
                        .filter(|f| f.required)
                        .map(|f| f.name.clone())
                        .collect::<BTreeSet<_>>();
                    if required.is_subset(columns) {
                        let known = schema
                            .fields
                            .iter()
                            .map(|f| f.name.as_str())
                            .collect::<BTreeSet<_>>();
                        let overlap = columns
                            .iter()
                            .filter(|column| known.contains(column.as_str()))
                            .count();
                        matches.insert(
                            (overlap, required.len()),
                            (input.alias.clone(), input.schema_ref.clone()),
                        );
                    }
                }
            }
        }
        matches.into_iter().next_back().map(|(_, value)| value)
    }

    fn materialize_tree(&self, record: &UploadBatchRecordSet) -> Result<PathBuf, UploadBatchError> {
        let root = self
            .data_dir
            .join("upload-work")
            .join(record.batch.id.to_string())
            .join("input");
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root)?;
        for file in &record.files {
            let relative = file.relative_path.as_deref().unwrap_or(&file.file_name);
            let destination = root.join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)?
            }
            std::fs::copy(self.data_dir.join(&file.staged_path), destination)?;
        }
        Ok(root)
    }
    async fn commit_path(
        &self,
        scope: i32,
        path: &Path,
    ) -> Result<super::ArtifactRef, UploadBatchError> {
        let staged = self
            .artifacts
            .store()
            .stage(std::fs::File::open(path)?, StageLimits::default())
            .map_err(|e| UploadBatchError::Invalid(e.code().into(), e.to_string()))?;
        Ok(self
            .artifacts
            .commit(scope, staged, ArtifactMetadata::parquet())
            .await?)
    }
    async fn release_previous(&self, scope: i32, record: &UploadBatchRecordSet) {
        for artifact in record
            .candidates
            .iter()
            .filter_map(|candidate| candidate.normalized_artifact_id)
        {
            let _ = self.artifacts.release(scope, artifact).await;
        }
    }
    async fn release_ids(&self, scope: i32, artifacts: &[i32]) {
        for artifact in artifacts {
            let _ = self.artifacts.release(scope, *artifact).await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn candidate(
    batch: i32,
    file: &str,
    sheet: Option<String>,
    display: &str,
    alias: Option<&str>,
    purpose: Option<CandidatePurpose>,
    schema_ref: Option<String>,
    artifact: i32,
    metadata: &crate::storage::ParquetMetadata,
    evidence: serde_json::Value,
) -> NewUploadCandidate {
    let schema = serde_json::to_value(&metadata.columns).unwrap();
    let schema_json = schema.to_string();
    NewUploadCandidate {
        id: stable_id(
            "candidate",
            &format!("{batch}:{file}:{}", sheet.as_deref().unwrap_or(display)),
        ),
        batch_id: batch,
        file_id: file.into(),
        sheet_name: sheet,
        display_name: display.into(),
        proposed_alias: alias.map(str::to_string),
        normalized_alias: alias.map(normalize_alias),
        alias_explicit: false,
        purpose: purpose.map(|p| p.as_str().into()),
        inferred_schema_ref: schema_ref,
        normalized_artifact_id: Some(artifact),
        schema_hash: Some(hex::encode(Sha256::digest(schema_json.as_bytes()))),
        schema_json: Some(schema_json),
        row_count: i64::try_from(metadata.row_count).ok(),
        evidence_json: evidence.to_string(),
        selected: true,
        created_at_ms: now_ms(),
    }
}
fn view(record: UploadBatchRecordSet) -> UploadBatchView {
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
        diagnostics: serde_json::from_str(&record.batch.diagnostics_json).unwrap_or_default(),
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
fn extension(name: &str) -> String {
    Path::new(name)
        .extension()
        .and_then(|v| v.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}
fn stable_id(prefix: &str, value: &str) -> String {
    format!(
        "{prefix}_{}",
        &hex::encode(Sha256::digest(value.as_bytes()))[..24]
    )
}
fn source_fingerprint<'a>(
    files: impl IntoIterator<Item = (&'a String, &'a String, Option<&'a str>, &'a str)>,
) -> String {
    let mut rows = files
        .into_iter()
        .map(|(id, name, path, sha)| format!("{id}\0{name}\0{}\0{sha}", path.unwrap_or_default()))
        .collect::<Vec<_>>();
    rows.sort();
    hex::encode(Sha256::digest(rows.join("\n").as_bytes()))
}
fn batch_diagnostics(
    record: &UploadBatchRecordSet,
    settings: &ParseSettings,
    error: Option<&UploadBatchError>,
) -> serde_json::Value {
    let source_hash = source_fingerprint(record.files.iter().map(|file| {
        (
            &file.client_file_id,
            &file.file_name,
            file.relative_path.as_deref(),
            file.sha256.as_str(),
        )
    }));
    serde_json::json!({"sourceHash":source_hash,"parserVersion":"v4.1","parseSpec":settings,"errors":error.map(|value|vec![serde_json::json!({"code":value.code(),"message":value.to_string()})]).unwrap_or_default()})
}
fn normalize_alias(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}
fn ambiguous_delimiter(bytes: &[u8]) -> bool {
    let first = String::from_utf8_lossy(bytes)
        .lines()
        .next()
        .unwrap_or("")
        .to_string();
    [',', ';', '\t']
        .into_iter()
        .filter(|c| first.contains(*c))
        .count()
        > 1
}
