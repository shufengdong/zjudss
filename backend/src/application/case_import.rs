use std::sync::Arc;

use sha2::{Digest, Sha256};

use crate::auth::CurrentUser;
use crate::db::models::DraftRef;
use crate::engine_resources::{EngineExample, EngineResourceSnapshot};
use crate::storage::StageLimits;

use super::{ModelImportError, ModelImportService, ModelImportView, UploadBatch, UploadFile};

#[derive(Debug)]
pub enum CaseImportError {
    NotFound,
    SnapshotMismatch,
    NotModelizable,
    SourceChanged,
    Io(std::io::Error),
    ModelImport(ModelImportError),
}

impl std::fmt::Display for CaseImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "案例不存在"),
            Self::SnapshotMismatch => write!(formatter, "引擎资源快照不匹配"),
            Self::NotModelizable => write!(formatter, "案例不能转换为 DataFrame 模型"),
            Self::SourceChanged => write!(formatter, "案例源文件已变化"),
            Self::Io(error) => write!(formatter, "{error}"),
            Self::ModelImport(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for CaseImportError {}
impl From<ModelImportError> for CaseImportError {
    fn from(value: ModelImportError) -> Self {
        Self::ModelImport(value)
    }
}

#[derive(Clone)]
pub struct CaseImportService {
    snapshot: Arc<EngineResourceSnapshot>,
    model_imports: ModelImportService,
    max_file_bytes: u64,
}

impl CaseImportService {
    pub fn new(
        snapshot: Arc<EngineResourceSnapshot>,
        model_imports: ModelImportService,
        max_file_bytes: u64,
    ) -> Self {
        Self {
            snapshot,
            model_imports,
            max_file_bytes,
        }
    }

    pub fn is_modelizable(example: &EngineExample) -> bool {
        example.importable
            && example.manifest.is_some()
            && example
                .files
                .iter()
                .any(|file| file.virtual_path.to_ascii_lowercase().ends_with(".dss"))
    }

    pub async fn accept(
        &self,
        user: &CurrentUser,
        session_id: i32,
        example_id: &str,
        snapshot_id: &str,
        client_import_id: String,
        expected_draft: Option<DraftRef>,
    ) -> Result<ModelImportView, CaseImportError> {
        if snapshot_id != self.snapshot.id {
            return Err(CaseImportError::SnapshotMismatch);
        }
        let example = self
            .snapshot
            .examples
            .iter()
            .find(|example| example.id == example_id)
            .ok_or(CaseImportError::NotFound)?;
        if !Self::is_modelizable(example) {
            return Err(CaseImportError::NotModelizable);
        }
        let manifest = example
            .manifest
            .as_ref()
            .ok_or(CaseImportError::NotModelizable)?;
        let mut verified = Vec::with_capacity(example.files.len());
        for source in &example.files {
            let path = safe_snapshot_file(&self.snapshot.engine_root, &source.source_path)?;
            let bytes = std::fs::read(path).map_err(CaseImportError::Io)?;
            if format!("{:x}", Sha256::digest(&bytes)) != source.sha256 {
                return Err(CaseImportError::SourceChanged);
            }
            verified.push((source, bytes));
        }

        let mut files = Vec::with_capacity(verified.len());
        for (index, (source, bytes)) in verified.into_iter().enumerate() {
            let staged = self
                .model_imports
                .artifact_store()
                .stage(
                    std::io::Cursor::new(bytes),
                    StageLimits {
                        max_bytes: self.max_file_bytes,
                    },
                )
                .map_err(|error| {
                    CaseImportError::ModelImport(ModelImportError::Invalid(error.to_string()))
                })?;
            files.push(UploadFile {
                client_file_id: format!("case-{index}"),
                file_name: std::path::Path::new(&source.virtual_path)
                    .file_name()
                    .and_then(|value| value.to_str())
                    .unwrap_or("case-file")
                    .to_string(),
                relative_path: Some(source.virtual_path.clone()),
                media_type: None,
                staged,
            });
        }
        self.model_imports
            .accept_case(
                user,
                session_id,
                client_import_id,
                expected_draft,
                UploadBatch {
                    client_batch_id: "case".into(),
                    files,
                },
                serde_json::json!({
                    "exampleId": example.id,
                    "title": example.title,
                    "categoryPath": example.category_path,
                    "bundlePath": example.bundle_path,
                    "engineResourceSnapshot": self.snapshot.id,
                    "manifestSchemaVersion": manifest.schema_version,
                    "sourceTreeHash": manifest.source.tree_sha256,
                    "caseTreeHash": example.tree_hash,
                    "fileCount": example.files.len(),
                }),
            )
            .await
            .map_err(Into::into)
    }

    pub async fn process(
        &self,
        user: &CurrentUser,
        import_id: i32,
    ) -> Result<ModelImportView, CaseImportError> {
        self.model_imports
            .process(user, import_id)
            .await
            .map_err(Into::into)
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
            .map_err(CaseImportError::Io)?
            .file_type()
            .is_symlink()
        {
            return Err(CaseImportError::SourceChanged);
        }
    }
    let canonical = current.canonicalize().map_err(CaseImportError::Io)?;
    if !canonical.starts_with(root) || !canonical.is_file() {
        return Err(CaseImportError::SourceChanged);
    }
    Ok(canonical)
}
