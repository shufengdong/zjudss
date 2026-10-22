use crate::application::now_ms;
use crate::auth::StorageScopeRef;
use crate::db::models::{Artifact, StorageScope};
use crate::db::repositories::{ArtifactDraft, ArtifactRepo, RepoError, StorageScopeRepo};
use crate::storage::{ArtifactStore, StagedArtifact, StorageError};

pub use crate::db::repositories::{StorageLimits, StorageUsage};

#[derive(Debug, Clone)]
pub struct ArtifactMetadata {
    pub format: String,
    pub media_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArtifactRef {
    pub id: i32,
    pub sha256: String,
    pub size_bytes: i64,
    pub relative_path: String,
}

impl From<Artifact> for ArtifactRef {
    fn from(value: Artifact) -> Self {
        Self {
            id: value.id,
            sha256: value.sha256,
            size_bytes: value.size_bytes,
            relative_path: value.storage_path,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GcReport {
    pub removed_artifacts: usize,
    pub failed_blobs: usize,
}

#[derive(Debug)]
pub enum ArtifactError {
    NotFound,
    QuotaExceeded,
    Storage(StorageError),
    Repository(RepoError),
    SizeOverflow,
}

impl std::fmt::Display for ArtifactError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "资源不存在"),
            Self::QuotaExceeded => write!(f, "存储配额不足"),
            Self::Storage(error) => write!(f, "{error}"),
            Self::Repository(error) => write!(f, "{error}"),
            Self::SizeOverflow => write!(f, "artifact 大小超出支持范围"),
        }
    }
}

impl std::error::Error for ArtifactError {}

impl From<RepoError> for ArtifactError {
    fn from(value: RepoError) -> Self {
        match value {
            RepoError::NotFound => Self::NotFound,
            RepoError::QuotaExceeded => Self::QuotaExceeded,
            other => Self::Repository(other),
        }
    }
}

impl From<StorageError> for ArtifactError {
    fn from(value: StorageError) -> Self {
        Self::Storage(value)
    }
}

#[derive(Clone)]
pub struct ArtifactService {
    store: ArtifactStore,
    artifacts: ArtifactRepo,
    scopes: StorageScopeRepo,
}

impl ArtifactService {
    pub fn new(store: ArtifactStore, artifacts: ArtifactRepo, scopes: StorageScopeRepo) -> Self {
        Self {
            store,
            artifacts,
            scopes,
        }
    }

    pub fn store(&self) -> &ArtifactStore {
        &self.store
    }

    pub async fn resolve_scope(
        &self,
        reference: &StorageScopeRef,
    ) -> Result<StorageScope, ArtifactError> {
        self.scopes
            .resolve(reference, now_ms())
            .await
            .map_err(Into::into)
    }

    pub async fn commit(
        &self,
        scope_id: i32,
        staged: StagedArtifact,
        metadata: ArtifactMetadata,
    ) -> Result<ArtifactRef, ArtifactError> {
        let size_bytes =
            i64::try_from(staged.size_bytes).map_err(|_| ArtifactError::SizeOverflow)?;
        let relative_path = ArtifactStore::relative_path(&staged.sha256);
        let draft = ArtifactDraft {
            sha256: staged.sha256.clone(),
            size_bytes,
            storage_path: relative_path,
            format: metadata.format,
            media_type: metadata.media_type,
            created_at_ms: now_ms(),
        };
        let store = self.store.clone();
        let mut staged = staged;
        self.artifacts
            .commit_with_publish(scope_id, draft, move |path| {
                store
                    .publish(&mut staged, path)
                    .map_err(|error| RepoError::Storage(error.to_string()))
            })
            .await
            .map(ArtifactRef::from)
            .map_err(Into::into)
    }

    pub async fn get_usage(&self, scope_id: i32) -> Result<StorageUsage, ArtifactError> {
        self.scopes.usage(scope_id).await.map_err(Into::into)
    }

    pub async fn find_owned(
        &self,
        scope_id: i32,
        artifact_id: i32,
    ) -> Result<crate::db::models::Artifact, ArtifactError> {
        self.artifacts
            .find_owned(scope_id, artifact_id)
            .await
            .map_err(Into::into)
    }

    pub async fn set_limits(
        &self,
        scope_id: i32,
        limits: StorageLimits,
    ) -> Result<(), ArtifactError> {
        self.scopes
            .set_limits(scope_id, limits, now_ms())
            .await
            .map_err(Into::into)
    }

    pub async fn release(&self, scope_id: i32, artifact_id: i32) -> Result<(), ArtifactError> {
        self.artifacts
            .release(scope_id, artifact_id, now_ms())
            .await
            .map_err(Into::into)
    }

    pub async fn collect_unreferenced(&self) -> Result<GcReport, ArtifactError> {
        let mut report = GcReport {
            removed_artifacts: 0,
            failed_blobs: 0,
        };
        for failure in self.artifacts.gc_failures().await? {
            match self.store.remove(&failure.relative_path) {
                Ok(()) => {
                    self.artifacts
                        .clear_gc_failure(failure.relative_path)
                        .await?;
                    report.removed_artifacts += 1;
                }
                Err(error) => {
                    report.failed_blobs += 1;
                    self.artifacts
                        .record_gc_failure(failure.relative_path, error.to_string(), now_ms())
                        .await?;
                }
            }
        }
        let artifacts = self.artifacts.take_unreferenced().await?;
        for artifact in artifacts {
            match self.store.remove(&artifact.storage_path) {
                Ok(()) => report.removed_artifacts += 1,
                Err(error) => {
                    report.failed_blobs += 1;
                    self.artifacts
                        .record_gc_failure(artifact.storage_path, error.to_string(), now_ms())
                        .await?;
                }
            }
        }
        Ok(report)
    }
}
