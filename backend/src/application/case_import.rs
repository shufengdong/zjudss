use std::sync::Arc;

use crate::auth::CurrentUser;
use crate::engine_resources::{EngineExample, EngineResourceSnapshot};
use crate::storage::StageLimits;

use super::{UploadBatchError, UploadBatchService, UploadBatchView, UploadFile, UploadSourceKind};

#[derive(Debug)]
pub enum CaseImportError {
    NotFound,
    SnapshotMismatch,
    NotModelizable,
    SourceChanged,
    Upload(UploadBatchError),
}

impl std::fmt::Display for CaseImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "案例不存在"),
            Self::SnapshotMismatch => write!(formatter, "引擎资源快照不匹配"),
            Self::NotModelizable => write!(formatter, "案例不能转换为 DataFrame 模型"),
            Self::SourceChanged => write!(formatter, "案例源文件已变化"),
            Self::Upload(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for CaseImportError {}
impl From<UploadBatchError> for CaseImportError {
    fn from(value: UploadBatchError) -> Self {
        Self::Upload(value)
    }
}

#[derive(Clone)]
pub struct CaseImportService {
    snapshot: Arc<EngineResourceSnapshot>,
    uploads: UploadBatchService,
    max_file_bytes: u64,
}

impl CaseImportService {
    pub fn new(
        snapshot: Arc<EngineResourceSnapshot>,
        uploads: UploadBatchService,
        max_file_bytes: u64,
    ) -> Self {
        Self {
            snapshot,
            uploads,
            max_file_bytes,
        }
    }

    pub async fn apply(
        &self,
        user: &CurrentUser,
        session_id: i32,
        example_id: String,
        snapshot_id: String,
        client_batch_id: String,
    ) -> Result<UploadBatchView, CaseImportError> {
        if snapshot_id != self.snapshot.id {
            return Err(CaseImportError::SnapshotMismatch);
        }
        let example = self
            .snapshot
            .examples
            .iter()
            .find(|item| item.id == example_id)
            .ok_or(CaseImportError::NotFound)?;
        if !Self::is_modelizable(example) {
            return Err(CaseImportError::NotModelizable);
        }
        let mut files = Vec::with_capacity(example.files.len());
        for (index, source) in example.files.iter().enumerate() {
            let path = safe_snapshot_file(&self.snapshot.engine_root, &source.source_path)?;
            let staged = self
                .uploads
                .artifact_store()
                .stage(
                    std::fs::File::open(path).map_err(|_| CaseImportError::SourceChanged)?,
                    StageLimits {
                        max_bytes: self.max_file_bytes,
                    },
                )
                .map_err(|e| {
                    CaseImportError::Upload(UploadBatchError::Invalid(
                        e.code().into(),
                        e.to_string(),
                    ))
                })?;
            if staged.sha256 != source.sha256 {
                return Err(CaseImportError::SourceChanged);
            }
            files.push(UploadFile {
                client_file_id: format!("case-{index}"),
                file_name: std::path::Path::new(&source.virtual_path)
                    .file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("case-file")
                    .into(),
                relative_path: Some(source.virtual_path.clone()),
                media_type: None,
                staged,
            });
        }
        self.uploads
            .accept(
                user,
                session_id,
                client_batch_id,
                UploadSourceKind::Case,
                files,
            )
            .await
            .map_err(Into::into)
    }

    pub fn is_modelizable(example: &EngineExample) -> bool {
        example.importable
            && example.manifest.as_ref().is_some_and(|manifest| {
                manifest.projection.status == "enabled"
                    && manifest.projection.managed_entry == "managed.rss"
            })
            && example
                .files
                .iter()
                .any(|file| file.virtual_path.to_ascii_lowercase().ends_with(".dss"))
    }
}

fn safe_snapshot_file(
    root: &std::path::Path,
    relative: &str,
) -> Result<std::path::PathBuf, CaseImportError> {
    let relative =
        crate::ingestion::safe_relative(relative).map_err(|_| CaseImportError::SourceChanged)?;
    let mut current = root.to_path_buf();
    for part in std::path::Path::new(&relative).components() {
        current.push(part.as_os_str());
        if std::fs::symlink_metadata(&current)
            .map_err(|_| CaseImportError::SourceChanged)?
            .file_type()
            .is_symlink()
        {
            return Err(CaseImportError::SourceChanged);
        }
    }
    let canonical = current
        .canonicalize()
        .map_err(|_| CaseImportError::SourceChanged)?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(CaseImportError::SourceChanged);
    }
    Ok(canonical)
}
