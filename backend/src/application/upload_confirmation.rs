use std::collections::BTreeSet;
use std::fs::File;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::auth::CurrentUser;
use crate::db::models::DraftRef;
use crate::db::repositories::{
    DataResourceRepo, RepoError, ResourcePublishInput, UploadConfirmationRepo, UploadRepo,
};
use crate::storage::StageLimits;

use super::data_resources::{stable_resource_id, view};
use super::resource_alias::{default_alias, normalize_alias};
use super::upload_candidate_policy::validate_replacement_action;
use super::{
    ArtifactError, ArtifactMetadata, ArtifactService, CandidateDecision, CandidatePurpose,
    DataResourceError, DataResourceService, DataResourceView, UploadBatchService, now_ms,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReplacementAction {
    Replace,
    SaveAsNew,
    Cancel,
}

impl ReplacementAction {
    pub(super) fn as_str(self) -> &'static str {
        match self {
            Self::Replace => "replace",
            Self::SaveAsNew => "save_as_new",
            Self::Cancel => "cancel",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadConfirmation {
    pub upload_batch_id: String,
    pub status: String,
    pub model_status: String,
    pub resource_status: String,
    pub model_receipt: Option<Value>,
    pub resource_receipt: Option<Value>,
    pub resources: Vec<DataResourceView>,
    pub diagnostics: Vec<Value>,
}

struct PublishRequest {
    batch_id: i32,
    decisions: Vec<CandidateDecision>,
    action: Option<ReplacementAction>,
    expected_draft: Option<DraftRef>,
    confirmation: Option<ConfirmationClaim>,
}

struct ConfirmationClaim {
    operation_id: String,
    payload_hash: String,
}

pub type UploadConfirmationError = DataResourceError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PublicationStrategy {
    OrdinaryUpload,
    CaseSwitch,
}

impl PublicationStrategy {
    fn for_source(source_kind: &str) -> Self {
        if source_kind == "case" {
            Self::CaseSwitch
        } else {
            Self::OrdinaryUpload
        }
    }
}

#[derive(Clone)]
pub struct UploadConfirmationService {
    repo: DataResourceRepo,
    confirmation_repo: UploadConfirmationRepo,
    upload_repo: UploadRepo,
    uploads: UploadBatchService,
    resources: DataResourceService,
    artifacts: ArtifactService,
    data_dir: PathBuf,
}

impl UploadConfirmationService {
    pub fn new(
        repo: DataResourceRepo,
        confirmation_repo: UploadConfirmationRepo,
        upload_repo: UploadRepo,
        uploads: UploadBatchService,
        resources: DataResourceService,
        artifacts: ArtifactService,
        data_dir: PathBuf,
    ) -> Self {
        Self {
            repo,
            confirmation_repo,
            upload_repo,
            uploads,
            resources,
            artifacts,
            data_dir,
        }
    }

    pub async fn publish_candidates(
        &self,
        user: &CurrentUser,
        batch_id: i32,
        decisions: Vec<CandidateDecision>,
    ) -> Result<UploadConfirmation, DataResourceError> {
        self.publish(
            user,
            PublishRequest {
                batch_id,
                decisions,
                action: None,
                expected_draft: None,
                confirmation: None,
            },
        )
        .await
    }

    pub async fn publish_replacement(
        &self,
        user: &CurrentUser,
        batch_id: i32,
        decisions: Vec<CandidateDecision>,
        action: ReplacementAction,
    ) -> Result<UploadConfirmation, DataResourceError> {
        self.publish(
            user,
            PublishRequest {
                batch_id,
                decisions,
                action: Some(action),
                expected_draft: None,
                confirmation: None,
            },
        )
        .await
    }

    pub async fn confirm(
        &self,
        user: &CurrentUser,
        batch_id: i32,
        decisions: Vec<CandidateDecision>,
        action: Option<ReplacementAction>,
        expected_draft: Option<crate::db::models::DraftRef>,
        client_operation_id: &str,
    ) -> Result<UploadConfirmation, DataResourceError> {
        if client_operation_id.trim().is_empty() || client_operation_id.len() > 128 {
            return Err(Self::invalid(
                "idempotency_conflict",
                "clientOperationId 必须为 1..=128 个字符",
            ));
        }
        let mut batch = self.upload_repo.find(&user.id, batch_id).await?;
        if action == Some(ReplacementAction::Cancel) {
            self.uploads.discard(user, batch_id).await?;
            return self.confirmation(user, batch_id, vec![]).await;
        }
        let payload_hash = confirmation_payload_hash(&decisions, action, expected_draft);
        if batch.batch.confirm_operation_id.as_deref() == Some(client_operation_id) {
            if batch.batch.confirm_payload_hash.as_deref() != Some(payload_hash.as_str()) {
                return Err(Self::invalid(
                    "idempotency_conflict",
                    "同一确认操作不能更换请求内容",
                ));
            }
            if batch.batch.confirmation_started {
                if batch.batch.confirmation_in_progress {
                    return self
                        .wait_for_confirmation(user, batch_id, client_operation_id)
                        .await;
                }
                if matches!(
                    batch.batch.status.as_str(),
                    "ready_to_confirm" | "partially_confirmed" | "confirmed"
                ) {
                    return self.confirmation(user, batch_id, vec![]).await;
                }
            }
        } else if batch.batch.confirmation_in_progress || batch.batch.status == "confirmed" {
            return Err(Self::invalid(
                "idempotency_conflict",
                "上传批次已由另一确认操作认领",
            ));
        }
        if batch.batch.expires_at_ms <= now_ms() {
            return Err(Self::invalid("upload_batch_expired", "上传批次已过期"));
        }
        self.confirmation_repo
            .claim_confirmation(
                &user.id,
                batch_id,
                client_operation_id,
                &payload_hash,
                now_ms(),
            )
            .await?;
        batch = self.upload_repo.find(&user.id, batch_id).await?;
        if batch.batch.confirm_operation_id.as_deref() == Some(client_operation_id)
            && batch.batch.confirm_payload_hash.as_deref() == Some(payload_hash.as_str())
            && batch.batch.confirmation_started
        {
            if batch.batch.confirmation_in_progress {
                return self
                    .wait_for_confirmation(user, batch_id, client_operation_id)
                    .await;
            }
            if matches!(
                batch.batch.status.as_str(),
                "ready_to_confirm" | "partially_confirmed" | "confirmed"
            ) {
                return self.confirmation(user, batch_id, vec![]).await;
            }
        }
        if !matches!(
            batch.batch.status.as_str(),
            "ready_to_confirm" | "partially_confirmed" | "confirmed"
        ) && !(batch.batch.status == "awaiting_purpose" && !decisions.is_empty())
        {
            return Err(Self::invalid(
                "upload_state_invalid",
                "上传批次尚未完成解析和用途确认",
            ));
        }
        self.publish(
            user,
            PublishRequest {
                batch_id,
                decisions,
                action,
                expected_draft,
                confirmation: Some(ConfirmationClaim {
                    operation_id: client_operation_id.to_owned(),
                    payload_hash,
                }),
            },
        )
        .await
    }

    async fn publish(
        &self,
        user: &CurrentUser,
        request: PublishRequest,
    ) -> Result<UploadConfirmation, DataResourceError> {
        let PublishRequest {
            batch_id,
            mut decisions,
            action,
            expected_draft,
            confirmation,
        } = request;
        let confirmation_operation_id = confirmation
            .as_ref()
            .map(|claim| claim.operation_id.as_str());
        let confirmation_payload_hash = confirmation
            .as_ref()
            .map(|claim| claim.payload_hash.as_str());
        let before = self.upload_repo.find(&user.id, batch_id).await?;
        let explicit_aliases = decisions
            .iter()
            .filter(|decision| {
                let Some(alias) = decision.alias.as_deref() else {
                    return false;
                };
                before
                    .candidates
                    .iter()
                    .find(|candidate| candidate.id == decision.candidate_id)
                    .and_then(|candidate| candidate.proposed_alias.as_deref())
                    .is_none_or(|suggested| normalize_alias(suggested) != normalize_alias(alias))
            })
            .map(|decision| decision.candidate_id.clone())
            .collect::<BTreeSet<_>>();
        for decision in &mut decisions {
            if decision.selected
                && decision.purpose == CandidatePurpose::DataResource
                && decision.alias.is_none()
                && let Some(candidate) = before
                    .candidates
                    .iter()
                    .find(|candidate| candidate.id == decision.candidate_id)
            {
                decision.alias = Some(
                    candidate
                        .proposed_alias
                        .clone()
                        .unwrap_or_else(|| default_alias(&candidate.display_name)),
                );
            }
        }
        if before.batch.status == "confirmed" {
            return self.confirmation(user, batch_id, vec![]).await;
        }
        if !matches!(
            before.batch.status.as_str(),
            "ready_to_confirm" | "partially_confirmed"
        ) && !(before.batch.status == "awaiting_purpose" && !decisions.is_empty())
        {
            return Err(Self::invalid(
                "upload_state_invalid",
                "上传批次尚未完成解析和用途确认",
            ));
        }
        if validate_replacement_action(
            before.batch.replacement_resource_id.is_some(),
            action.is_some(),
        )
        .is_err()
        {
            return Err(Self::invalid(
                "replacement_preview_invalid",
                "替换上传必须明确选择覆盖、另存为或取消",
            ));
        }
        if action == Some(ReplacementAction::Replace)
            && let Some(resource_id) = before.batch.replacement_resource_id.as_deref()
        {
            let old = self.repo.find(&user.id, resource_id, false).await?;
            for decision in &mut decisions {
                if decision.selected
                    && decision.purpose == CandidatePurpose::DataResource
                    && decision.alias.is_none()
                {
                    decision.alias = Some(old.resource.alias.clone());
                }
            }
        }
        for decision in &decisions {
            if decision.selected
                && let Some(alias) = decision.alias.as_deref().or(decision.table_name.as_deref())
            {
                self.validate_alias(
                    alias,
                    decision.purpose,
                    before
                        .candidates
                        .iter()
                        .find(|candidate| candidate.id == decision.candidate_id)
                        .and_then(|candidate| candidate.inferred_schema_ref.as_deref()),
                )?;
            }
        }
        let decision_update = if decisions.is_empty() {
            Ok(None)
        } else if let (Some(operation_id), Some(payload_hash)) =
            (confirmation_operation_id, confirmation_payload_hash)
        {
            self.uploads
                .confirm_candidates(
                    user,
                    batch_id,
                    decisions.clone(),
                    operation_id,
                    payload_hash,
                    explicit_aliases,
                )
                .await
                .map(Some)
        } else {
            self.uploads
                .update_candidates_with_alias_intent(
                    user,
                    batch_id,
                    decisions.clone(),
                    explicit_aliases,
                )
                .await
                .map(Some)
        };
        if let Err(error) = decision_update {
            let latest = self.upload_repo.find(&user.id, batch_id).await?;
            if let (Some(operation_id), Some(payload_hash)) =
                (confirmation_operation_id, confirmation_payload_hash)
                && latest.batch.confirm_operation_id.as_deref() == Some(operation_id)
                && latest.batch.confirm_payload_hash.as_deref() == Some(payload_hash)
            {
                if latest.batch.confirmation_in_progress {
                    return self
                        .wait_for_confirmation(user, batch_id, operation_id)
                        .await;
                }
                if latest.batch.confirmation_started
                    && matches!(
                        latest.batch.status.as_str(),
                        "ready_to_confirm" | "partially_confirmed" | "confirmed"
                    )
                {
                    return self.confirmation(user, batch_id, vec![]).await;
                }
            }
            return Err(error.into());
        }
        if decisions.is_empty()
            && let (Some(operation_id), Some(payload_hash)) =
                (confirmation_operation_id, confirmation_payload_hash)
            && let Err(error) = self
                .confirmation_repo
                .begin_confirmation(&user.id, batch_id, operation_id, payload_hash, now_ms())
                .await
        {
            let latest = self.upload_repo.find(&user.id, batch_id).await?;
            if latest.batch.confirm_operation_id.as_deref() == Some(operation_id)
                && latest.batch.confirm_payload_hash.as_deref() == Some(payload_hash)
            {
                if latest.batch.confirmation_in_progress {
                    return self
                        .wait_for_confirmation(user, batch_id, operation_id)
                        .await;
                }
                if latest.batch.confirmation_started
                    && matches!(
                        latest.batch.status.as_str(),
                        "ready_to_confirm" | "partially_confirmed" | "confirmed"
                    )
                {
                    return self.confirmation(user, batch_id, vec![]).await;
                }
            }
            return Err(error.into());
        }
        let result = async {
        let record = self.upload_repo.find(&user.id, batch_id).await?;
        let strategy = PublicationStrategy::for_source(&record.batch.source_kind);
        if confirmation_operation_id.is_none() && record.batch.expires_at_ms <= now_ms() {
            return Err(DataResourceError::Invalid(
                "upload_batch_expired",
                "上传批次已过期".into(),
            ));
        }
        if let Some(ReplacementAction::Cancel) = action {
            self.uploads.discard(user, batch_id).await?;
            return Ok(UploadConfirmation {
                upload_batch_id: batch_id.to_string(),
                status: "discarded".into(),
                model_status: record.batch.model_publish_status,
                resource_status: "not_applicable".into(),
                model_receipt: None,
                resource_receipt: None,
                resources: vec![],
                diagnostics: vec![],
            });
        }
        self.commit_originals(user, &record).await?;
        let refreshed = self.upload_repo.find(&user.id, batch_id).await?;
        if strategy == PublicationStrategy::OrdinaryUpload
            && refreshed.batch.model_publish_status != "not_applicable"
            && refreshed.batch.model_publish_status != "published"
        {
            match self
                .confirmation_repo
                .apply_model_candidates_to_draft(&user.id, batch_id, expected_draft, now_ms())
                .await
            {
                Ok(()) => {}
                Err(error) => {
                    let code = match &error {
                        RepoError::InvalidState(message)
                            if message == "draft_revision_conflict" =>
                        {
                            "draft_revision_conflict"
                        }
                        _ => "model_publish_failed",
                    };
                    let diagnostic = serde_json::json!({"code":code,"message":error.to_string()});
                    self.confirmation_repo
                        .mark_child_failed(
                            &user.id,
                            batch_id,
                            "model",
                            serde_json::json!([diagnostic]).to_string(),
                            now_ms(),
                        )
                        .await?;
                }
            }
        }
        let refreshed = self.upload_repo.find(&user.id, batch_id).await?;
        let mut published_rows = Vec::new();
        if (refreshed.batch.resource_publish_status != "not_applicable"
            && refreshed.batch.resource_publish_status != "published")
            || (strategy == PublicationStrategy::CaseSwitch
                && refreshed.batch.model_publish_status != "published")
        {
            let inputs = refreshed.candidates.iter().filter(|candidate| candidate.selected && candidate.purpose.as_deref() == Some("data_resource")).map(|candidate| {
                let proposed = candidate.normalized_alias.clone().or_else(|| candidate.proposed_alias.as_deref().map(normalize_alias)).unwrap_or_else(|| default_alias(&candidate.display_name));
                ResourcePublishInput {
                    candidate_id: candidate.id.clone(),
                    resource_id: stable_resource_id(batch_id, &candidate.id),
                    alias: candidate.proposed_alias.clone().unwrap_or_else(|| proposed.clone()),
                    normalized_alias: proposed,
                    display_name: candidate.display_name.clone(),
                    source_kind: if candidate.evidence_json.contains("controlled_mapping") {
                        "mapping".into()
                    } else if candidate.evidence_json.contains("static_import") {
                        "static_import".into()
                    } else {
                        "upload".into()
                    },
                    lineage_json: if candidate.evidence_json.contains("controlled_mapping") {
                        candidate.evidence_json.clone()
                    } else {
                        serde_json::json!({"uploadBatchId":batch_id,"candidateId":candidate.id,"sourceKind":refreshed.batch.source_kind}).to_string()
                    },
                    alias_explicit: candidate.alias_explicit,
                }
            }).collect::<Vec<_>>();
            let replacement = refreshed.batch.replacement_resource_id.clone().map(|id| {
                (
                    id,
                    action
                        .unwrap_or(ReplacementAction::Replace)
                        .as_str()
                        .to_string(),
                )
            });
            let published = match strategy {
                PublicationStrategy::CaseSwitch => {
                    self.confirmation_repo
                        .switch_case(&user.id, batch_id, inputs, expected_draft, now_ms())
                        .await
                }
                PublicationStrategy::OrdinaryUpload => {
                    self.confirmation_repo
                        .publish_resources(&user.id, batch_id, inputs, replacement, now_ms())
                        .await
                }
            };
            match published {
                Ok(result) => {
                    published_rows = result.resources;
                }
                Err(error) => {
                    let code = match &error {
                        RepoError::InvalidState(message)
                            if message == "resource_alias_conflict" =>
                        {
                            "resource_alias_conflict"
                        }
                        _ => "resource_publish_failed",
                    };
                    let message = if code == "resource_alias_conflict" {
                        let mut existing = self.repo.list_current_for_user(&user.id).await?;
                        if strategy == PublicationStrategy::CaseSwitch {
                            let preview: Value = serde_json::from_str(&refreshed.batch.diagnostics_json).unwrap_or_default();
                            if let Some(retiring) = preview["caseSwitch"]["retiringResources"].as_array() {
                                existing.retain(|resource| !retiring.iter().any(|old| old["id"].as_str() == Some(resource.id.as_str())));
                            }
                        }
                        let conflicts = refreshed.candidates.iter()
                            .filter(|candidate| candidate.selected && candidate.purpose.as_deref() == Some("data_resource"))
                            .filter_map(|candidate| {
                                let alias = candidate.normalized_alias.clone()
                                    .or_else(|| candidate.proposed_alias.as_deref().map(normalize_alias))
                                    .unwrap_or_else(|| default_alias(&candidate.display_name));
                                existing.iter().find(|resource| resource.session_id == refreshed.batch.session_id && resource.normalized_alias == alias)
                                    .map(|_| format!("{}（别名：{}）", candidate.display_name, alias))
                            }).collect::<Vec<_>>();
                        format!("以下数据资源与当前会话已有资源重名：{}。请修改别名后重试数据发布；已发布的模型无需重复发布。", conflicts.join("、"))
                    } else { error.to_string() };
                    let diagnostic = serde_json::json!({"code":code,"message":message});
                    if strategy == PublicationStrategy::CaseSwitch {
                        self.confirmation_repo.mark_child_failed(&user.id, batch_id, "model", serde_json::json!([diagnostic.clone()]).to_string(), now_ms()).await?;
                    }
                    self.confirmation_repo
                        .mark_child_failed(
                            &user.id,
                            batch_id,
                            "resource",
                            serde_json::json!([diagnostic]).to_string(),
                            now_ms(),
                        )
                        .await?;
                }
            }
        }
        let cleanup = self
            .confirmation_repo
            .finish_batch(&user.id, batch_id, now_ms())
            .await?;
        for path in cleanup.staged_paths {
            let _ = std::fs::remove_file(self.data_dir.join(path));
        }
        let mut views = Vec::new();
        for row in published_rows {
            views.push(self.get(user, &row.id).await?);
        }
        self.confirmation(user, batch_id, views).await
        }
        .await;
        if result.is_err()
            && let Some(operation_id) = confirmation_operation_id
        {
            let _ = self
                .confirmation_repo
                .end_confirmation(&user.id, batch_id, operation_id, now_ms())
                .await;
        }
        result
    }

    async fn wait_for_confirmation(
        &self,
        user: &CurrentUser,
        batch_id: i32,
        operation_id: &str,
    ) -> Result<UploadConfirmation, DataResourceError> {
        for _ in 0..300 {
            let current = self.upload_repo.find(&user.id, batch_id).await?;
            if current.batch.confirm_operation_id.as_deref() != Some(operation_id) {
                return Err(Self::invalid(
                    "idempotency_conflict",
                    "上传批次已由另一确认操作认领",
                ));
            }
            if !current.batch.confirmation_in_progress {
                return self.confirmation(user, batch_id, vec![]).await;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        Err(Self::invalid(
            "upload_state_invalid",
            "相同确认操作仍在执行，请稍后重试",
        ))
    }

    async fn confirmation(
        &self,
        user: &CurrentUser,
        batch_id: i32,
        resources: Vec<DataResourceView>,
    ) -> Result<UploadConfirmation, DataResourceError> {
        let record = self.upload_repo.find(&user.id, batch_id).await?;
        let resources = if resources.is_empty() {
            self.repo
                .for_batch(&user.id, batch_id)
                .await?
                .into_iter()
                .map(view)
                .collect()
        } else {
            resources
        };
        let mut diagnostics =
            serde_json::from_str::<Vec<Value>>(&record.batch.model_diagnostics_json)
                .unwrap_or_default();
        diagnostics.extend(
            serde_json::from_str::<Vec<Value>>(&record.batch.resource_diagnostics_json)
                .unwrap_or_default(),
        );
        diagnostics.dedup();
        Ok(UploadConfirmation {
            upload_batch_id: batch_id.to_string(),
            status: record.batch.status,
            model_status: record.batch.model_publish_status,
            resource_status: record.batch.resource_publish_status,
            model_receipt: record
                .batch
                .model_receipt_json
                .as_deref()
                .and_then(|receipt| serde_json::from_str(receipt).ok()),
            resource_receipt: record
                .batch
                .resource_receipt_json
                .as_deref()
                .and_then(|receipt| serde_json::from_str(receipt).ok()),
            resources,
            diagnostics,
        })
    }

    async fn commit_originals(
        &self,
        user: &CurrentUser,
        record: &crate::db::repositories::UploadBatchRecordSet,
    ) -> Result<(), DataResourceError> {
        let needed_files = record
            .candidates
            .iter()
            .filter(|candidate| {
                candidate.selected && candidate.purpose.as_deref() == Some("data_resource")
            })
            .map(|candidate| candidate.file_id.as_str())
            .collect::<BTreeSet<_>>();
        if needed_files.is_empty() {
            return Ok(());
        }
        for upload in &record.files {
            if !needed_files.contains(upload.id.as_str()) || upload.source_artifact_id.is_some() {
                continue;
            }
            let current = self.upload_repo.find(&user.id, record.batch.id).await?;
            if current.batch.resource_publish_status == "published" {
                return Ok(());
            }
            if !matches!(
                current.batch.status.as_str(),
                "ready_to_confirm" | "partially_confirmed"
            ) {
                return Err(Self::invalid(
                    "upload_state_invalid",
                    "上传批次已结束，不能继续提交原文件",
                ));
            }
            let source = self.data_dir.join(&upload.staged_path);
            let source_file = match File::open(source) {
                Ok(file) => file,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    let current = self.upload_repo.find(&user.id, record.batch.id).await?;
                    if current.batch.resource_publish_status == "published" {
                        return Ok(());
                    }
                    return Err(error.into());
                }
                Err(error) => return Err(error.into()),
            };
            let staged = self
                .artifacts
                .store()
                .stage(
                    source_file,
                    StageLimits {
                        max_bytes: u64::try_from(upload.size_bytes).unwrap_or(u64::MAX),
                    },
                )
                .map_err(ArtifactError::from)?;
            let format = std::path::Path::new(&upload.file_name)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("binary")
                .to_ascii_lowercase();
            let committed = self
                .artifacts
                .commit_upload_source(
                    user.storage_scope_id,
                    upload.id.clone(),
                    staged,
                    ArtifactMetadata {
                        format,
                        media_type: upload.media_type.clone(),
                    },
                )
                .await;
            if let Err(ArtifactError::Repository(RepoError::InvalidState(message))) = &committed
                && message == "upload_state_invalid"
            {
                let current = self.upload_repo.find(&user.id, record.batch.id).await?;
                if current.batch.resource_publish_status == "published" {
                    return Ok(());
                }
            }
            committed?;
        }
        Ok(())
    }

    async fn get(
        &self,
        user: &CurrentUser,
        id: &str,
    ) -> Result<DataResourceView, DataResourceError> {
        self.resources.get(user, id).await
    }

    fn validate_alias(
        &self,
        alias: &str,
        purpose: CandidatePurpose,
        schema_ref: Option<&str>,
    ) -> Result<(), DataResourceError> {
        self.resources.validate_alias(alias, purpose, schema_ref)
    }

    fn invalid(code: &'static str, message: &str) -> DataResourceError {
        DataResourceError::Invalid(code, message.into())
    }
}

fn confirmation_payload_hash(
    decisions: &[CandidateDecision],
    action: Option<ReplacementAction>,
    expected_draft: Option<DraftRef>,
) -> String {
    let mut decisions = decisions.to_vec();
    decisions.sort_by(|left, right| left.candidate_id.cmp(&right.candidate_id));
    let payload = serde_json::json!({
        "decisions": decisions,
        "replacementAction": action,
        "expectedDraft": expected_draft,
    });
    hex::encode(Sha256::digest(payload.to_string().as_bytes()))
}
