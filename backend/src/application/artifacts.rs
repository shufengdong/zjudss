use std::fs::File;
use std::sync::Arc;

use tokio::sync::RwLock;

use crate::application::now_ms;
use crate::auth::UserId;
use crate::db::models::{Artifact, StorageScope};
use crate::db::repositories::{ArtifactDraft, ArtifactRepo, RepoError, StorageScopeRepo};
use crate::db::repositories::{ChartViewDraft, ChartViewRepo, PublishedChartView};
use crate::storage::{ArtifactStore, StagedArtifact, StorageError};

pub use crate::db::repositories::{StorageLimits, StorageUsage};

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsageSummary {
    pub scope: String,
    pub artifact_limit: i64,
    pub byte_limit: i64,
    pub warning_ratio: f64,
    pub artifact_count: i64,
    pub logical_bytes: i64,
    pub is_warning: bool,
    pub is_exceeded: bool,
}

#[derive(Debug, Clone)]
pub struct ArtifactMetadata {
    pub format: String,
    pub media_type: Option<String>,
}

impl ArtifactMetadata {
    pub(crate) fn parquet() -> Self {
        Self {
            format: "parquet".into(),
            media_type: Some("application/vnd.apache.parquet".into()),
        }
    }

    pub(crate) fn into_draft(
        self,
        staged: &StagedArtifact,
        created_at_ms: i64,
    ) -> Result<ArtifactDraft, ArtifactError> {
        Ok(ArtifactDraft {
            sha256: staged.sha256.clone(),
            size_bytes: i64::try_from(staged.size_bytes)
                .map_err(|_| ArtifactError::SizeOverflow)?,
            storage_path: ArtifactStore::relative_path(&staged.sha256),
            format: self.format,
            media_type: self.media_type,
            created_at_ms,
        })
    }
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
    access_gate: Arc<RwLock<()>>,
}

impl ArtifactService {
    pub fn new(store: ArtifactStore, artifacts: ArtifactRepo, scopes: StorageScopeRepo) -> Self {
        Self {
            store,
            artifacts,
            scopes,
            access_gate: Arc::new(RwLock::new(())),
        }
    }

    pub fn store(&self) -> &ArtifactStore {
        &self.store
    }

    pub async fn resolve_scope(&self, scope_id: i32) -> Result<StorageScope, ArtifactError> {
        self.scopes.find(scope_id).await.map_err(Into::into)
    }

    pub async fn commit(
        &self,
        scope_id: i32,
        staged: StagedArtifact,
        metadata: ArtifactMetadata,
    ) -> Result<ArtifactRef, ArtifactError> {
        let draft = metadata.into_draft(&staged, now_ms())?;
        let store = self.store.clone();
        let artifacts = self.artifacts.clone();
        let access_gate = self.access_gate.clone();
        let mut staged = staged;
        tokio::spawn(async move {
            let _guard = access_gate.read_owned().await;
            artifacts
                .commit_with_publish(scope_id, draft, move |path| {
                    store
                        .publish(&mut staged, path)
                        .map_err(|error| RepoError::Storage(error.to_string()))
                })
                .await
                .map(ArtifactRef::from)
                .map_err(Into::into)
        })
        .await
        .map_err(|error| ArtifactError::Repository(RepoError::Join(error)))?
    }

    pub async fn publish_chart_view(
        &self,
        repo: ChartViewRepo,
        user: UserId,
        mut draft: ChartViewDraft,
        staged: StagedArtifact,
    ) -> Result<PublishedChartView, ArtifactError> {
        draft.artifact = ArtifactMetadata::parquet().into_draft(&staged, draft.created_at_ms)?;
        let store = self.store.clone();
        let access_gate = self.access_gate.clone();
        let mut staged = staged;
        tokio::spawn(async move {
            let _guard = access_gate.read_owned().await;
            repo.publish(&user, draft, move |path| {
                store
                    .publish(&mut staged, path)
                    .map_err(|error| RepoError::Storage(error.to_string()))
            })
            .await
            .map_err(Into::into)
        })
        .await
        .map_err(|error| ArtifactError::Repository(RepoError::Join(error)))?
    }

    pub async fn commit_upload_source(
        &self,
        scope_id: i32,
        upload_file_id: String,
        staged: StagedArtifact,
        metadata: ArtifactMetadata,
    ) -> Result<ArtifactRef, ArtifactError> {
        let draft = metadata.into_draft(&staged, now_ms())?;
        let store = self.store.clone();
        let artifacts = self.artifacts.clone();
        let access_gate = self.access_gate.clone();
        let mut staged = staged;
        tokio::spawn(async move {
            let _guard = access_gate.read_owned().await;
            artifacts
                .commit_upload_source_with_publish(scope_id, draft, upload_file_id, move |path| {
                    store
                        .publish(&mut staged, path)
                        .map_err(|error| RepoError::Storage(error.to_string()))
                })
                .await
                .map(ArtifactRef::from)
                .map_err(Into::into)
        })
        .await
        .map_err(|error| ArtifactError::Repository(RepoError::Join(error)))?
    }

    pub async fn get_usage(&self, scope_id: i32) -> Result<StorageUsage, ArtifactError> {
        self.scopes.usage(scope_id).await.map_err(Into::into)
    }

    pub async fn usage_for(&self, scope_id: i32) -> Result<StorageUsageSummary, ArtifactError> {
        let scope = self.resolve_scope(scope_id).await?;
        let usage = self.get_usage(scope.id).await?;
        let artifact_ratio = usage.used_artifacts as f64 / usage.artifact_limit as f64;
        let byte_ratio = usage.used_bytes as f64 / usage.byte_limit as f64;
        Ok(StorageUsageSummary {
            scope: "user".to_string(),
            artifact_limit: usage.artifact_limit,
            byte_limit: usage.byte_limit,
            warning_ratio: usage.warning_ratio,
            artifact_count: usage.used_artifacts,
            logical_bytes: usage.used_bytes,
            is_warning: artifact_ratio >= usage.warning_ratio || byte_ratio >= usage.warning_ratio,
            is_exceeded: usage.used_artifacts >= usage.artifact_limit
                || usage.used_bytes >= usage.byte_limit,
        })
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

    pub async fn open_owned(&self, scope_id: i32, artifact_id: i32) -> Result<File, ArtifactError> {
        let _guard = self.access_gate.read().await;
        let artifact = self.find_owned(scope_id, artifact_id).await?;
        self.store.open(&artifact.storage_path).map_err(Into::into)
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
        let _guard = self.access_gate.write().await;
        self.artifacts
            .release(scope_id, artifact_id, now_ms())
            .await
            .map_err(Into::into)
    }

    pub(crate) async fn exclusive_publication_guard(
        &self,
    ) -> tokio::sync::OwnedRwLockWriteGuard<()> {
        self.access_gate.clone().write_owned().await
    }

    pub async fn collect_unreferenced(&self) -> Result<GcReport, ArtifactError> {
        let _guard = self.access_gate.write().await;
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
        let known = self
            .artifacts
            .storage_paths()
            .await?
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();
        for path in self.store.object_paths()? {
            if !known.contains(&path) {
                match self.store.remove(&path) {
                    Ok(()) => report.removed_artifacts += 1,
                    Err(error) => {
                        report.failed_blobs += 1;
                        self.artifacts
                            .record_gc_failure(path, error.to_string(), now_ms())
                            .await?;
                    }
                }
            }
        }
        Ok(report)
    }
}

#[cfg(test)]
mod tests {
    use std::io::Cursor;

    use crate::storage::{ArtifactStore, StageLimits};

    use super::ArtifactMetadata;

    #[test]
    fn parquet_metadata_is_canonical() {
        let metadata = ArtifactMetadata::parquet();

        assert_eq!(metadata.format, "parquet");
        assert_eq!(
            metadata.media_type.as_deref(),
            Some("application/vnd.apache.parquet")
        );
    }

    #[test]
    fn staged_artifact_metadata_builds_the_complete_commit_draft() {
        let root = tempfile::tempdir().expect("create artifact test directory");
        let store = ArtifactStore::new(root.path().to_path_buf()).expect("create artifact store");
        let staged = store
            .stage(Cursor::new(b"draft bytes"), StageLimits::default())
            .expect("stage artifact");

        let draft = ArtifactMetadata {
            format: "binary".to_string(),
            media_type: Some("application/octet-stream".to_string()),
        }
        .into_draft(&staged, 1_700_000_000_123)
        .expect("build artifact draft");

        let sha256 = "e45a4fb4ff1c334d36c8210bb341bd2ebd8a20e7b63013c1bbbd04439c886fa4";
        assert_eq!(draft.sha256, sha256);
        assert_eq!(draft.size_bytes, 11);
        assert_eq!(draft.storage_path, format!("objects/e4/{sha256}"));
        assert_eq!(draft.format, "binary");
        assert_eq!(
            draft.media_type.as_deref(),
            Some("application/octet-stream")
        );
        assert_eq!(draft.created_at_ms, 1_700_000_000_123);
    }
}
