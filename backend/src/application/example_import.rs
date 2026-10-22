use std::sync::Arc;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::auth::Principal;
use crate::db::models::ModelVersion;
use crate::engine_resources::{EngineExample, EngineResourceSnapshot};
use crate::storage::StageLimits;

use super::{
    ArtifactError, ArtifactMetadata, ArtifactRef, ArtifactService, ModelVersionStatus, SourceKind,
    VersionDraft, VersionInputDraft, VersionOrigin, WorkspaceError, WorkspaceService,
};

#[derive(Debug)]
pub enum ExampleImportError {
    NotFound,
    SnapshotMismatch,
    ExpectedVersionRequired,
    NotImportable(Vec<String>),
    SourceChanged(String),
    Artifact(ArtifactError),
    Workspace(WorkspaceError),
    Io(std::io::Error),
}

impl std::fmt::Display for ExampleImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "案例不存在"),
            Self::SnapshotMismatch => write!(f, "引擎资源快照不匹配"),
            Self::ExpectedVersionRequired => write!(f, "expectedDraftVersionId 必填"),
            Self::NotImportable(items) => write!(f, "案例不可导入: {}", items.join("; ")),
            Self::SourceChanged(path) => write!(f, "案例源文件已变化: {path}"),
            Self::Artifact(error) => write!(f, "{error}"),
            Self::Workspace(error) => write!(f, "{error}"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}
impl std::error::Error for ExampleImportError {}
impl ExampleImportError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "example_not_found",
            Self::SnapshotMismatch => "engine_resource_snapshot_mismatch",
            Self::ExpectedVersionRequired => "expected_version_required",
            Self::NotImportable(_) => "example_not_importable",
            Self::SourceChanged(_) => "example_source_changed",
            Self::Artifact(_) => "artifact_error",
            Self::Workspace(WorkspaceError::VersionConflict) => "version_conflict",
            Self::Workspace(_) => "workspace_error",
            Self::Io(_) => "example_io_error",
        }
    }
}
impl From<ArtifactError> for ExampleImportError {
    fn from(value: ArtifactError) -> Self {
        Self::Artifact(value)
    }
}
impl From<WorkspaceError> for ExampleImportError {
    fn from(value: WorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

#[derive(Clone)]
pub struct ExampleImportService {
    snapshot: Arc<EngineResourceSnapshot>,
    artifacts: ArtifactService,
    workspaces: WorkspaceService,
    max_file_bytes: u64,
}

impl ExampleImportService {
    pub fn new(
        snapshot: Arc<EngineResourceSnapshot>,
        artifacts: ArtifactService,
        workspaces: WorkspaceService,
        max_file_bytes: u64,
    ) -> Self {
        Self {
            snapshot,
            artifacts,
            workspaces,
            max_file_bytes,
        }
    }

    pub fn snapshot(&self) -> &EngineResourceSnapshot {
        &self.snapshot
    }

    pub async fn import(
        &self,
        principal: &Principal,
        session_id: i32,
        example_id: &str,
        snapshot_id: &str,
        expected_base: Option<i32>,
    ) -> Result<ModelVersion, ExampleImportError> {
        if snapshot_id != self.snapshot.id {
            return Err(ExampleImportError::SnapshotMismatch);
        }
        let example = self
            .snapshot
            .examples
            .iter()
            .find(|item| item.id == example_id)
            .ok_or(ExampleImportError::NotFound)?;
        if !example.importable {
            return Err(ExampleImportError::NotImportable(
                example.diagnostics.clone(),
            ));
        }
        let workspace = self
            .workspaces
            .get_or_create(&principal.access, session_id)
            .await?;
        if workspace
            .workspace
            .draft_version_id
            .or(workspace.workspace.active_version_id)
            .is_some()
            && expected_base.is_none()
        {
            return Err(ExampleImportError::ExpectedVersionRequired);
        }
        for version in self
            .workspaces
            .list_versions(&principal.access, session_id)
            .await?
        {
            let manifest: Value = serde_json::from_str(&version.manifest_json).unwrap_or_default();
            if manifest.get("origin").is_some_and(|origin| {
                origin.get("exampleId").and_then(Value::as_str) == Some(example_id)
                    && origin.get("engineResourceSnapshot").and_then(Value::as_str)
                        == Some(snapshot_id)
                    && origin.get("sourceTreeHash").and_then(Value::as_str)
                        == Some(example.tree_hash.as_str())
            }) {
                return Ok(version);
            }
        }
        let current = workspace
            .workspace
            .draft_version_id
            .or(workspace.workspace.active_version_id);
        if current != expected_base {
            return Err(WorkspaceError::VersionConflict.into());
        }

        let storage_scope = self.artifacts.resolve_scope(&principal.storage).await?;
        let mut committed = Vec::<ArtifactRef>::new();
        let result = self
            .copy_files(storage_scope.id, example, &mut committed)
            .await;
        let inputs = match result {
            Ok(inputs) => inputs,
            Err(error) => {
                release_all(&self.artifacts, storage_scope.id, &committed).await;
                return Err(error);
            }
        };
        let source_kind = if example.source_kind == "rssbundle" {
            SourceKind::Rssbundle
        } else {
            SourceKind::Rss
        };
        match self
            .workspaces
            .publish_version(
                &principal.access,
                workspace.workspace.id,
                expected_base,
                VersionDraft {
                    status: ModelVersionStatus::Ready,
                    source_kind,
                    origin: VersionOrigin::Example {
                        example_id: example.id.clone(),
                        engine_resource_snapshot: self.snapshot.id.clone(),
                        source_tree_hash: example.tree_hash.clone(),
                    },
                    manifest: json!({
                        "entryPath": example.entry_path,
                        "studyType": example.study_type,
                        "solver": example.solver,
                        "requiredFeatures": example.required_features,
                        "diagnostics": [],
                        "missing": [],
                    }),
                    patch: None,
                    inputs,
                },
            )
            .await
        {
            Ok(version) => Ok(version),
            Err(error) => {
                release_all(&self.artifacts, storage_scope.id, &committed).await;
                Err(error.into())
            }
        }
    }

    async fn copy_files(
        &self,
        scope_id: i32,
        example: &EngineExample,
        committed: &mut Vec<ArtifactRef>,
    ) -> Result<Vec<VersionInputDraft>, ExampleImportError> {
        let mut inputs = Vec::with_capacity(example.files.len());
        for source in &example.files {
            let path = safe_snapshot_file(&self.snapshot.engine_root, &source.source_path)?;
            let bytes = std::fs::read(&path).map_err(ExampleImportError::Io)?;
            if format!("{:x}", Sha256::digest(&bytes)) != source.sha256 {
                return Err(ExampleImportError::SourceChanged(
                    source.virtual_path.clone(),
                ));
            }
            let staged = self
                .artifacts
                .store()
                .stage(
                    std::io::Cursor::new(bytes),
                    StageLimits {
                        max_bytes: self.max_file_bytes,
                    },
                )
                .map_err(ArtifactError::from)?;
            let format = std::path::Path::new(&source.virtual_path)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("file")
                .to_ascii_lowercase();
            let artifact = self
                .artifacts
                .commit(
                    scope_id,
                    staged,
                    ArtifactMetadata {
                        format,
                        media_type: None,
                    },
                )
                .await?;
            committed.push(artifact.clone());
            let is_entry = source.source_path.ends_with(&example.entry_path)
                || source.virtual_path == example.entry_path
                || example.entry_path.ends_with(&source.virtual_path);
            inputs.push(VersionInputDraft {
                artifact_id: artifact.id,
                sha256: artifact.sha256,
                role: if is_entry {
                    "entry".into()
                } else if source.virtual_path == "manifest.json" {
                    "manifest".into()
                } else {
                    "data".into()
                },
                relative_path: Some(source.virtual_path.clone()),
                display_name: std::path::Path::new(&source.virtual_path)
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("file")
                    .to_string(),
                source_artifact_id: None,
                source_artifact_sha256: None,
                sheet_name: None,
                binding: None,
            });
        }
        Ok(inputs)
    }
}

fn safe_snapshot_file(
    root: &std::path::Path,
    relative: &str,
) -> Result<std::path::PathBuf, ExampleImportError> {
    let relative = crate::ingestion::safe_relative(relative)
        .map_err(|_| ExampleImportError::SourceChanged(relative.to_string()))?;
    let mut current = root.to_path_buf();
    for part in std::path::Path::new(&relative).components() {
        current.push(part.as_os_str());
        let metadata = std::fs::symlink_metadata(&current).map_err(ExampleImportError::Io)?;
        if metadata.file_type().is_symlink() {
            return Err(ExampleImportError::SourceChanged(relative));
        }
    }
    let canonical = current.canonicalize().map_err(ExampleImportError::Io)?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(ExampleImportError::SourceChanged(relative));
    }
    Ok(canonical)
}

async fn release_all(artifacts: &ArtifactService, scope_id: i32, committed: &[ArtifactRef]) {
    for artifact in committed {
        let _ = artifacts.release(scope_id, artifact.id).await;
    }
}
