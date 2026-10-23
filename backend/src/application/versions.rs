use rand_core::{OsRng, RngCore};
use sha2::{Digest, Sha256};

use crate::adapters::ENGINE_CONTRACT_VERSION;
use crate::auth::CurrentUser;
use crate::db::models::DraftRef;
use crate::db::repositories::{
    DeletionImpact, VersionRepo, VersionRepoError, VersionSnapshot, WorkspaceSnapshot,
};

use super::{ArtifactError, ArtifactService, ms_to_iso, now_ms};

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishDraftRequest {
    pub expected_draft_id: i32,
    pub expected_draft_revision: i64,
    pub client_publish_id: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivateVersionRequest {
    #[serde(default)]
    pub discard_draft: bool,
    pub expected_draft: Option<DraftRef>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDeletionPreview {
    pub impact_token: String,
    pub run_count: usize,
    pub result_count: usize,
    pub reclaimable_bytes: i64,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct PublishedVersion {
    pub version: VersionSnapshot,
    pub deduplicated: bool,
    pub replayed: bool,
}

#[derive(Debug)]
pub enum VersionError {
    NotFound,
    DraftRevisionConflict,
    DraftRequired,
    DraftDiscardConfirmationRequired,
    ActiveVersion,
    Busy,
    ImpactChanged,
    InvalidImpactToken,
    EmptyDraft,
    Invalid(String),
    Artifact(ArtifactError),
    Repository(crate::db::repositories::RepoError),
}

impl std::fmt::Display for VersionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "资源不存在"),
            Self::DraftRevisionConflict => write!(formatter, "草稿已变化，请刷新后重试"),
            Self::DraftRequired => write!(formatter, "没有可发布的草稿"),
            Self::DraftDiscardConfirmationRequired => {
                write!(formatter, "激活历史版本前必须明确放弃当前草稿")
            }
            Self::ActiveVersion => write!(formatter, "当前激活版本不能删除"),
            Self::Busy => write!(formatter, "版本仍有运行中的消费者"),
            Self::ImpactChanged => write!(formatter, "删除影响已变化，请重新确认"),
            Self::InvalidImpactToken => write!(formatter, "删除确认已失效"),
            Self::EmptyDraft => write!(formatter, "空草稿不能发布"),
            Self::Invalid(message) => write!(formatter, "{message}"),
            Self::Artifact(error) => write!(formatter, "{error}"),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for VersionError {}

impl From<VersionRepoError> for VersionError {
    fn from(value: VersionRepoError) -> Self {
        match value {
            VersionRepoError::Repository(crate::db::repositories::RepoError::NotFound) => {
                Self::NotFound
            }
            VersionRepoError::Repository(error) => Self::Repository(error),
            VersionRepoError::DraftConflict => Self::DraftRevisionConflict,
            VersionRepoError::DraftRequired => Self::DraftRequired,
            VersionRepoError::DraftPresent => Self::DraftDiscardConfirmationRequired,
            VersionRepoError::ActiveVersion => Self::ActiveVersion,
            VersionRepoError::Busy => Self::Busy,
            VersionRepoError::ImpactChanged => Self::ImpactChanged,
            VersionRepoError::InvalidToken => Self::InvalidImpactToken,
            VersionRepoError::EmptyDraft => Self::EmptyDraft,
        }
    }
}

#[derive(Clone)]
pub struct VersionService {
    repo: VersionRepo,
    artifacts: ArtifactService,
}

impl VersionService {
    pub fn new(repo: VersionRepo, artifacts: ArtifactService) -> Self {
        Self { repo, artifacts }
    }

    pub async fn workspace(
        &self,
        user: &CurrentUser,
        session_id: i32,
    ) -> Result<WorkspaceSnapshot, VersionError> {
        self.repo
            .workspace(&user.id, session_id, now_ms())
            .await
            .map_err(Into::into)
    }

    pub async fn versions(
        &self,
        user: &CurrentUser,
        session_id: i32,
    ) -> Result<Vec<VersionSnapshot>, VersionError> {
        self.repo.workspace(&user.id, session_id, now_ms()).await?;
        self.repo
            .list_versions(&user.id, session_id)
            .await
            .map_err(Into::into)
    }

    pub async fn version(
        &self,
        user: &CurrentUser,
        version_id: i32,
    ) -> Result<VersionSnapshot, VersionError> {
        self.repo
            .find_version(&user.id, version_id)
            .await
            .map_err(Into::into)
    }

    pub async fn publish(
        &self,
        user: &CurrentUser,
        session_id: i32,
        request: PublishDraftRequest,
    ) -> Result<PublishedVersion, VersionError> {
        if request.client_publish_id.trim().is_empty() || request.client_publish_id.len() > 128 {
            return Err(VersionError::Invalid(
                "clientPublishId 必须为 1 到 128 个字符".into(),
            ));
        }
        let result = self
            .repo
            .publish(
                &user.id,
                session_id,
                DraftRef {
                    id: request.expected_draft_id,
                    revision: request.expected_draft_revision,
                },
                request.client_publish_id,
                ENGINE_CONTRACT_VERSION.into(),
                now_ms(),
            )
            .await?;
        for artifact_id in result.release_artifact_ids {
            let _ = self
                .artifacts
                .release(user.storage_scope_id, artifact_id)
                .await;
        }
        let version = self.repo.find_version(&user.id, result.version_id).await?;
        Ok(PublishedVersion {
            version,
            deduplicated: result.deduplicated,
            replayed: result.replayed,
        })
    }

    pub async fn activate(
        &self,
        user: &CurrentUser,
        version_id: i32,
        request: ActivateVersionRequest,
    ) -> Result<WorkspaceSnapshot, VersionError> {
        let (workspace, release) = self
            .repo
            .activate(
                &user.id,
                version_id,
                request.discard_draft,
                request.expected_draft,
                now_ms(),
            )
            .await?;
        for artifact_id in release {
            let _ = self
                .artifacts
                .release(user.storage_scope_id, artifact_id)
                .await;
        }
        Ok(workspace)
    }

    pub async fn preview_deletion(
        &self,
        user: &CurrentUser,
        version_id: i32,
    ) -> Result<VersionDeletionPreview, VersionError> {
        let token = random_token();
        let token_hash = format!("{:x}", Sha256::digest(token.as_bytes()));
        let now = now_ms();
        let expires = now + 5 * 60 * 1000;
        let DeletionImpact {
            run_ids,
            result_ids,
            reclaimable_bytes,
            ..
        } = self
            .repo
            .preview_deletion(&user.id, version_id, token_hash, expires, now)
            .await?;
        Ok(VersionDeletionPreview {
            impact_token: token,
            run_count: run_ids.len(),
            result_count: result_ids.len(),
            reclaimable_bytes,
            expires_at: ms_to_iso(expires),
        })
    }

    pub async fn delete(
        &self,
        user: &CurrentUser,
        version_id: i32,
        impact_token: String,
    ) -> Result<(), VersionError> {
        if impact_token.len() != 64 {
            return Err(VersionError::InvalidImpactToken);
        }
        let token_hash = format!("{:x}", Sha256::digest(impact_token.as_bytes()));
        let result = self
            .repo
            .delete_version(&user.id, version_id, token_hash, now_ms())
            .await?;
        for artifact_id in result.release_artifact_ids {
            let _ = self
                .artifacts
                .release(user.storage_scope_id, artifact_id)
                .await;
        }
        Ok(())
    }
}

fn random_token() -> String {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let mut token = String::with_capacity(64);
    use std::fmt::Write as _;
    for byte in bytes {
        let _ = write!(token, "{byte:02x}");
    }
    token
}
