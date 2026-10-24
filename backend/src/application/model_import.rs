use std::collections::BTreeSet;
use std::path::PathBuf;
use std::sync::Arc;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::adapters::{ImportInspectSpec, ImportSpec, SimEngine};
use crate::auth::CurrentUser;
use crate::db::models::{DraftRef, NewModelImport, NewModelImportCandidate};
use crate::db::repositories::{ModelImportRecord, ModelImportRepo, RepoError};
use crate::ingestion::{
    ArchiveExtractor, ArchiveLimits, normalize_csv_to_parquet, normalize_excel_to_parquet,
};
use crate::storage::{ParquetInspector, StageLimits};

use super::{ArtifactError, ArtifactMetadata, ArtifactService, now_ms};
use crate::storage::StagedArtifact;

pub struct UploadFile {
    pub client_file_id: String,
    pub file_name: String,
    pub relative_path: Option<String>,
    pub media_type: Option<String>,
    pub staged: StagedArtifact,
}

pub struct UploadBatch {
    pub client_batch_id: String,
    pub files: Vec<UploadFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelImportCandidateView {
    pub id: String,
    pub kind: String,
    pub display_name: String,
    pub table_name: Option<String>,
    pub schema: Option<serde_json::Value>,
    pub row_count: Option<i64>,
    pub selected: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelImportView {
    pub id: String,
    pub session_id: String,
    pub client_import_id: String,
    pub source_kind: String,
    pub base_draft: Option<DraftRef>,
    pub status: String,
    pub stages: serde_json::Value,
    pub diagnostics: serde_json::Value,
    pub candidates: Vec<ModelImportCandidateView>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug)]
pub enum ModelImportError {
    NotFound,
    Invalid(String),
    IdempotencyConflict,
    DraftRevisionConflict,
    Artifact(ArtifactError),
    Repository(RepoError),
}

impl std::fmt::Display for ModelImportError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "资源不存在"),
            Self::Invalid(message) => write!(formatter, "{message}"),
            Self::IdempotencyConflict => write!(formatter, "clientImportId 已用于不同内容"),
            Self::DraftRevisionConflict => write!(formatter, "草稿已变化，请刷新后重新确认"),
            Self::Artifact(error) => write!(formatter, "{error}"),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for ModelImportError {}
impl From<ArtifactError> for ModelImportError {
    fn from(value: ArtifactError) -> Self {
        Self::Artifact(value)
    }
}
impl From<RepoError> for ModelImportError {
    fn from(value: RepoError) -> Self {
        match value {
            RepoError::NotFound => Self::NotFound,
            RepoError::IdempotencyConflict => Self::IdempotencyConflict,
            other => Self::Repository(other),
        }
    }
}

#[derive(Clone)]
pub struct ModelImportService {
    repo: ModelImportRepo,
    artifacts: ArtifactService,
    engine: Arc<dyn SimEngine>,
    data_dir: PathBuf,
    archive_limits: ArchiveLimits,
}

impl ModelImportService {
    pub(crate) fn artifact_store(&self) -> &crate::storage::ArtifactStore {
        self.artifacts.store()
    }

    pub fn new(
        repo: ModelImportRepo,
        artifacts: ArtifactService,
        engine: Arc<dyn SimEngine>,
        data_dir: PathBuf,
        archive_limits: ArchiveLimits,
    ) -> Self {
        Self {
            repo,
            artifacts,
            engine,
            data_dir,
            archive_limits,
        }
    }

    pub async fn accept(
        &self,
        user: &CurrentUser,
        session_id: i32,
        client_import_id: String,
        expected_draft: Option<DraftRef>,
        batch: UploadBatch,
    ) -> Result<ModelImportView, ModelImportError> {
        if client_import_id.trim().is_empty() || batch.files.is_empty() {
            return Err(ModelImportError::Invalid(
                "clientImportId 和至少一个模型文件是必需的".into(),
            ));
        }
        let mut file_ids = BTreeSet::new();
        if batch.files.iter().any(|file| {
            file.client_file_id.trim().is_empty() || !file_ids.insert(file.client_file_id.clone())
        }) {
            return Err(ModelImportError::Invalid(
                "clientFileId 必须非空且唯一".into(),
            ));
        }
        let source_sha256 = source_hash(&batch);
        if let Some(existing) = self
            .repo
            .find_by_client_id(&user.id, session_id, &client_import_id)
            .await?
        {
            return if existing.import.source_sha256 == source_sha256 {
                Ok(view(existing))
            } else {
                Err(ModelImportError::IdempotencyConflict)
            };
        }
        let workspace_id = self
            .repo
            .ensure_workspace(&user.id, session_id, now_ms())
            .await?;
        let actual_draft = self.repo.current_draft(workspace_id).await?;
        if actual_draft != expected_draft {
            return Err(ModelImportError::DraftRevisionConflict);
        }
        let scope = self.artifacts.resolve_scope(user.storage_scope_id).await?;
        let mut committed = Vec::new();
        for file in batch.files {
            let UploadFile {
                client_file_id,
                file_name,
                relative_path,
                media_type,
                staged,
            } = file;
            let format = std::path::Path::new(&file_name)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("file")
                .to_ascii_lowercase();
            match self
                .artifacts
                .commit(
                    scope.id,
                    staged,
                    ArtifactMetadata {
                        format,
                        media_type: media_type.clone(),
                    },
                )
                .await
            {
                Ok(artifact) => committed.push((
                    client_file_id,
                    file_name,
                    relative_path,
                    media_type,
                    artifact,
                )),
                Err(error) => {
                    for (_, _, _, _, artifact) in &committed {
                        let _ = self.artifacts.release(scope.id, artifact.id).await;
                    }
                    return Err(error.into());
                }
            }
        }
        let now = now_ms();
        let stages = serde_json::json!([
            {"key":"accepted","label":"已接收","status":"completed"},
            {"key":"inspect","label":"检查来源","status":"pending"},
            {"key":"normalize","label":"生成 DataFrame","status":"pending"},
            {"key":"apply","label":"等待确认","status":"pending"}
        ]);
        let import = NewModelImport {
            workspace_id,
            session_id,
            client_import_id,
            source_kind: "upload".into(),
            source_summary_json: serde_json::json!({
                "fileCount": committed.len(),
                "displayNames": committed.iter().map(|(_, file_name, _, _, _)| file_name.clone()).collect::<Vec<_>>()
            })
            .to_string(),
            source_sha256,
            base_draft_id: actual_draft.map(|draft| draft.id),
            base_draft_revision: actual_draft.map(|draft| draft.revision),
            status: "accepted".into(),
            stages_json: stages.to_string(),
            diagnostics_json: "{}".into(),
            error_code: None,
            error_message: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let candidates = committed
            .iter()
            .map(
                |(client_file_id, file_name, relative_path, _media_type, artifact)| {
                    NewModelImportCandidate {
                        import_id: 0,
                        candidate_id: candidate_id(client_file_id, &artifact.sha256),
                        candidate_kind: "entry".into(),
                        display_name: relative_path.clone().unwrap_or_else(|| file_name.clone()),
                        table_name: None,
                        artifact_id: Some(artifact.id),
                        schema_json: None,
                        row_count: None,
                        content_hash: Some(artifact.sha256.clone()),
                        evidence_json: serde_json::json!({
                            "clientFileId": client_file_id,
                            "fileName": file_name,
                            "relativePath": relative_path,
                            "stagingSource": true
                        })
                        .to_string(),
                        selected: false,
                        created_at_ms: now,
                    }
                },
            )
            .collect();
        match self.repo.create(import, candidates).await {
            Ok(record) => Ok(view(record)),
            Err(error) => {
                for (_, _, _, _, artifact) in committed {
                    let _ = self.artifacts.release(scope.id, artifact.id).await;
                }
                Err(error.into())
            }
        }
    }

    pub async fn get(
        &self,
        user: &CurrentUser,
        import_id: i32,
    ) -> Result<ModelImportView, ModelImportError> {
        Ok(view(self.repo.find(&user.id, import_id).await?))
    }

    pub async fn accept_case(
        &self,
        user: &CurrentUser,
        session_id: i32,
        client_import_id: String,
        expected_draft: Option<DraftRef>,
        batch: UploadBatch,
        source_summary: serde_json::Value,
    ) -> Result<ModelImportView, ModelImportError> {
        let accepted = self
            .accept(user, session_id, client_import_id, expected_draft, batch)
            .await?;
        let import_id = accepted
            .id
            .parse::<i32>()
            .map_err(|_| ModelImportError::Invalid("模型导入 ID 无效".into()))?;
        self.repo
            .mark_case_source(import_id, source_summary.to_string(), now_ms())
            .await?;
        self.get(user, import_id).await
    }

    pub async fn process(
        &self,
        user: &CurrentUser,
        import_id: i32,
    ) -> Result<ModelImportView, ModelImportError> {
        let record = self.repo.find(&user.id, import_id).await?;
        if record.import.status != "accepted" {
            return Ok(view(record));
        }
        if !self
            .repo
            .claim_processing(import_id, processing_stages().to_string(), now_ms())
            .await?
        {
            return self.get(user, import_id).await;
        }
        let all_tabular = record.candidates.iter().all(|candidate| {
            std::path::Path::new(&candidate.display_name)
                .extension()
                .and_then(|value| value.to_str())
                .is_some_and(|value| {
                    matches!(
                        value.to_ascii_lowercase().as_str(),
                        "csv" | "xls" | "xlsx" | "parquet"
                    )
                })
        });
        let result = if all_tabular {
            self.process_tabular(user, &record).await
        } else {
            self.process_dss(user, &record).await
        };
        if let Err(error) = result {
            // A failed import must not retain any source artifacts. The import row and
            // diagnostics remain queryable, but its candidate set is emptied before
            // the terminal status is published so quota/accounting stays atomic.
            let artifact_ids = self
                .repo
                .replace_candidates_if_processing(import_id, Vec::new())
                .await?;
            if let Some(artifact_ids) = artifact_ids {
                let scope = self.artifacts.resolve_scope(user.storage_scope_id).await?;
                for artifact_id in artifact_ids.into_iter().collect::<BTreeSet<_>>() {
                    let _ = self.artifacts.release(scope.id, artifact_id).await;
                }
                let _ = std::fs::remove_dir_all(
                    self.data_dir
                        .join("model-import-work")
                        .join(import_id.to_string()),
                );
                self.repo
                    .set_status_if_processing(
                        import_id,
                        "failed",
                        failed_stages().to_string(),
                        "{}".into(),
                        Some(("model_import_unsupported".into(), error.to_string())),
                        now_ms(),
                    )
                    .await?;
            }
        }
        self.get(user, import_id).await
    }

    async fn process_tabular(
        &self,
        user: &CurrentUser,
        record: &ModelImportRecord,
    ) -> Result<(), ModelImportError> {
        let scope = self.artifacts.resolve_scope(user.storage_scope_id).await?;
        let work = self
            .data_dir
            .join("model-import-work")
            .join(record.import.id.to_string());
        std::fs::create_dir_all(&work)
            .map_err(|error| ModelImportError::Invalid(error.to_string()))?;
        let mut normalized = Vec::new();
        for source in &record.candidates {
            let artifact_id = source.artifact_id.ok_or_else(|| {
                ModelImportError::Invalid("import source artifact missing".into())
            })?;
            let artifact = self.artifacts.find_owned(scope.id, artifact_id).await?;
            let path = self
                .artifacts
                .store()
                .resolve(&artifact.storage_path)
                .map_err(ArtifactError::from)?;
            let extension = std::path::Path::new(&source.display_name)
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            let outputs = match extension.as_str() {
                "csv" => {
                    let output = work.join(format!("{}.parquet", source.candidate_id));
                    normalize_csv_to_parquet(&path, &output)
                        .map_err(|error| ModelImportError::Invalid(error.to_string()))?;
                    let table_name = std::path::Path::new(&source.display_name)
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("table")
                        .to_string();
                    vec![(table_name, output)]
                }
                "xls" | "xlsx" => normalize_excel_to_parquet(&path, &work)
                    .map_err(|error| ModelImportError::Invalid(error.to_string()))?
                    .into_iter()
                    .map(|sheet| (sheet.sheet_name, sheet.path))
                    .collect(),
                "parquet" => vec![(
                    std::path::Path::new(&source.display_name)
                        .file_stem()
                        .and_then(|value| value.to_str())
                        .unwrap_or("table")
                        .to_string(),
                    path,
                )],
                _ => {
                    let _ = &self.engine;
                    return Err(ModelImportError::Invalid(
                        "当前批次需要 DSS/bundle import 入口选择".into(),
                    ));
                }
            };
            for (table_name, output) in outputs {
                let metadata = ParquetInspector::metadata(&output)
                    .map_err(|error| ModelImportError::Invalid(error.to_string()))?;
                if !metadata.snappy && extension != "parquet" {
                    return Err(ModelImportError::Invalid(
                        "规范化 Parquet 必须使用 Snappy".into(),
                    ));
                }
                let staged = self
                    .artifacts
                    .store()
                    .stage(
                        std::fs::File::open(&output)
                            .map_err(|error| ModelImportError::Invalid(error.to_string()))?,
                        StageLimits::default(),
                    )
                    .map_err(ArtifactError::from)?;
                let artifact = self
                    .artifacts
                    .commit(
                        scope.id,
                        staged,
                        ArtifactMetadata {
                            format: "parquet".into(),
                            media_type: Some("application/vnd.apache.parquet".into()),
                        },
                    )
                    .await?;
                normalized.push(NewModelImportCandidate {
                    import_id: record.import.id,
                    candidate_id: candidate_id(&table_name, &artifact.sha256),
                    candidate_kind: "table".into(),
                    display_name: source.display_name.clone(),
                    table_name: Some(table_name),
                    artifact_id: Some(artifact.id),
                    schema_json: Some(
                        serde_json::to_string(&metadata.columns).unwrap_or_else(|_| "[]".into()),
                    ),
                    row_count: Some(metadata.row_count as i64),
                    content_hash: Some(artifact.sha256),
                    evidence_json: serde_json::json!({"normalized":true}).to_string(),
                    selected: true,
                    created_at_ms: now_ms(),
                });
            }
        }
        let mut names = BTreeSet::new();
        let duplicates = normalized.iter().any(|candidate| {
            candidate
                .table_name
                .as_ref()
                .is_some_and(|name| !names.insert(name.clone()))
        });
        let normalized_artifacts = normalized
            .iter()
            .filter_map(|candidate| candidate.artifact_id)
            .collect::<BTreeSet<_>>();
        let old = self
            .repo
            .replace_candidates_if_processing(record.import.id, normalized)
            .await?;
        let Some(old) = old else {
            for artifact_id in normalized_artifacts {
                let _ = self.artifacts.release(scope.id, artifact_id).await;
            }
            let _ = std::fs::remove_dir_all(work);
            return Ok(());
        };
        for artifact_id in old {
            let _ = self.artifacts.release(scope.id, artifact_id).await;
        }
        let status = if duplicates {
            "awaiting_resolution"
        } else {
            "ready_to_apply"
        };
        self.repo
            .set_status_if_processing(
                record.import.id,
                status,
                ready_stages().to_string(),
                serde_json::json!({
                    "resolution": if duplicates { "duplicate_table" } else { "confirm_apply" }
                })
                .to_string(),
                None,
                now_ms(),
            )
            .await?;
        let _ = std::fs::remove_dir_all(work);
        Ok(())
    }

    async fn process_dss(
        &self,
        user: &CurrentUser,
        record: &ModelImportRecord,
    ) -> Result<(), ModelImportError> {
        let scope = self.artifacts.resolve_scope(user.storage_scope_id).await?;
        let work = self
            .data_dir
            .join("model-import-work")
            .join(record.import.id.to_string());
        let input = work.join("input");
        let output = work.join("output");
        let zip_source = if record.candidates.len() == 1
            && record.candidates[0]
                .display_name
                .to_ascii_lowercase()
                .ends_with(".zip")
        {
            record.candidates[0].artifact_id
        } else {
            None
        };
        if zip_source.is_none() {
            std::fs::create_dir_all(&input)
                .map_err(|error| ModelImportError::Invalid(error.to_string()))?;
        }
        let mut raw_artifacts = Vec::new();
        for source in &record.candidates {
            let artifact_id = source.artifact_id.ok_or_else(|| {
                ModelImportError::Invalid("import source artifact missing".into())
            })?;
            let artifact = self.artifacts.find_owned(scope.id, artifact_id).await?;
            raw_artifacts.push(artifact_id);
            let source_path = self
                .artifacts
                .store()
                .resolve(&artifact.storage_path)
                .map_err(ArtifactError::from)?;
            if Some(artifact_id) == zip_source {
                ArchiveExtractor::extract(&source_path, &input, self.archive_limits)
                    .map_err(|error| ModelImportError::Invalid(error.to_string()))?;
                continue;
            }
            let relative = std::path::Path::new(&source.display_name);
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|part| !matches!(part, std::path::Component::Normal(_)))
            {
                return Err(ModelImportError::Invalid(
                    "模型来源包含不安全相对路径".into(),
                ));
            }
            let destination = input.join(relative);
            if let Some(parent) = destination.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| ModelImportError::Invalid(error.to_string()))?;
            }
            std::fs::copy(source_path, destination)
                .map_err(|error| ModelImportError::Invalid(error.to_string()))?;
        }
        let inspection = self
            .engine
            .inspect_model_import(ImportInspectSpec {
                input_tree: input.clone(),
                timeout: std::time::Duration::from_secs(120),
            })
            .await
            .map_err(|error| ModelImportError::Invalid(error.to_string()))?;
        if inspection.entry_candidates.len() != 1 {
            self.repo
                .set_status_if_processing(
                    record.import.id,
                    "awaiting_resolution",
                    processing_stages().to_string(),
                    serde_json::json!({
                        "resolution":"entry_selection",
                        "entryCandidates":inspection.entry_candidates
                    })
                    .to_string(),
                    None,
                    now_ms(),
                )
                .await?;
            return Ok(());
        }
        let imported = self
            .engine
            .import_model(ImportSpec {
                input_tree: input,
                entry_candidate_id: inspection.entry_candidates[0].id.clone(),
                result_dir: output,
                timeout: std::time::Duration::from_secs(120),
            })
            .await
            .map_err(|error| ModelImportError::Invalid(error.to_string()))?;
        let mut candidates = Vec::new();
        for dataset in imported.dataframes {
            let staged = self
                .artifacts
                .store()
                .stage(
                    std::fs::File::open(&dataset.path)
                        .map_err(|error| ModelImportError::Invalid(error.to_string()))?,
                    StageLimits::default(),
                )
                .map_err(ArtifactError::from)?;
            let artifact = self
                .artifacts
                .commit(
                    scope.id,
                    staged,
                    ArtifactMetadata {
                        format: "parquet".into(),
                        media_type: Some("application/vnd.apache.parquet".into()),
                    },
                )
                .await?;
            candidates.push(NewModelImportCandidate {
                import_id: record.import.id,
                candidate_id: candidate_id(&dataset.name, &artifact.sha256),
                candidate_kind: "table".into(),
                display_name: dataset.name.clone(),
                table_name: Some(dataset.name),
                artifact_id: Some(artifact.id),
                schema_json: Some(
                    serde_json::to_string(&dataset.metadata.columns)
                        .unwrap_or_else(|_| "[]".into()),
                ),
                row_count: Some(dataset.metadata.row_count as i64),
                content_hash: Some(artifact.sha256),
                evidence_json: serde_json::json!({
                    "profile":imported.profile,
                    "diagnostics":imported.diagnostics
                })
                .to_string(),
                selected: true,
                created_at_ms: now_ms(),
            });
        }
        let candidate_artifacts = candidates
            .iter()
            .filter_map(|candidate| candidate.artifact_id)
            .collect::<BTreeSet<_>>();
        let old = self
            .repo
            .replace_candidates_if_processing(record.import.id, candidates)
            .await?;
        let Some(old) = old else {
            for artifact_id in candidate_artifacts {
                let _ = self.artifacts.release(scope.id, artifact_id).await;
            }
            let _ = std::fs::remove_dir_all(work);
            return Ok(());
        };
        for artifact_id in old {
            let _ = self.artifacts.release(scope.id, artifact_id).await;
        }
        self.repo
            .set_status_if_processing(
                record.import.id,
                "ready_to_apply",
                ready_stages().to_string(),
                serde_json::json!({
                    "resolution":"confirm_apply",
                    "profile":imported.profile,
                    "entryCandidateId":inspection.entry_candidates[0].id
                })
                .to_string(),
                None,
                now_ms(),
            )
            .await?;
        let _ = raw_artifacts;
        let _ = std::fs::remove_dir_all(work);
        Ok(())
    }

    pub async fn resolve_and_apply(
        &self,
        user: &CurrentUser,
        import_id: i32,
        expected: Option<DraftRef>,
        candidate_ids: Vec<String>,
    ) -> Result<DraftRef, ModelImportError> {
        let (draft, replaced) = self
            .repo
            .apply_candidates(&user.id, import_id, expected, candidate_ids, now_ms())
            .await
            .map_err(|error| match error {
                RepoError::VersionConflict => ModelImportError::DraftRevisionConflict,
                other => ModelImportError::from(other),
            })?;
        for artifact_id in replaced {
            let _ = self
                .artifacts
                .release(user.storage_scope_id, artifact_id)
                .await;
        }
        Ok(draft)
    }

    pub async fn discard(
        &self,
        user: &CurrentUser,
        import_id: i32,
    ) -> Result<(), ModelImportError> {
        let artifacts = self
            .repo
            .discard(&user.id, import_id, now_ms())
            .await
            .map_err(|error| match error {
                RepoError::InvalidState(message) => ModelImportError::Invalid(message),
                other => ModelImportError::from(other),
            })?;
        for artifact_id in artifacts.into_iter().collect::<BTreeSet<_>>() {
            let _ = self
                .artifacts
                .release(user.storage_scope_id, artifact_id)
                .await;
        }
        Ok(())
    }
}

fn processing_stages() -> serde_json::Value {
    serde_json::json!([
        {"key":"accepted","label":"已接收","status":"completed"},
        {"key":"inspect","label":"检查来源","status":"running"},
        {"key":"normalize","label":"生成 DataFrame","status":"pending"},
        {"key":"apply","label":"等待确认","status":"pending"}
    ])
}

fn ready_stages() -> serde_json::Value {
    serde_json::json!([
        {"key":"accepted","label":"已接收","status":"completed"},
        {"key":"inspect","label":"检查来源","status":"completed"},
        {"key":"normalize","label":"生成 DataFrame","status":"completed"},
        {"key":"apply","label":"等待确认","status":"pending"}
    ])
}

fn failed_stages() -> serde_json::Value {
    serde_json::json!([
        {"key":"accepted","label":"已接收","status":"completed"},
        {"key":"inspect","label":"检查来源","status":"failed"},
        {"key":"normalize","label":"生成 DataFrame","status":"pending"},
        {"key":"apply","label":"等待确认","status":"pending"}
    ])
}

fn source_hash(batch: &UploadBatch) -> String {
    let mut values = batch
        .files
        .iter()
        .map(|file| {
            format!(
                "{}\0{}\0{}\0{}",
                file.client_file_id,
                file.file_name,
                file.relative_path.as_deref().unwrap_or_default(),
                file.staged.sha256
            )
        })
        .collect::<Vec<_>>();
    values.sort();
    hex::encode(Sha256::digest(values.join("\0").as_bytes()))
}

fn candidate_id(client_file_id: &str, sha256: &str) -> String {
    let digest = hex::encode(Sha256::digest(
        format!("{client_file_id}\0{sha256}").as_bytes(),
    ));
    digest[..24].to_string()
}

fn view(record: ModelImportRecord) -> ModelImportView {
    ModelImportView {
        id: record.import.id.to_string(),
        session_id: record.import.session_id.to_string(),
        client_import_id: record.import.client_import_id,
        source_kind: record.import.source_kind,
        base_draft: record
            .import
            .base_draft_id
            .zip(record.import.base_draft_revision)
            .map(|(id, revision)| DraftRef { id, revision }),
        status: record.import.status,
        stages: serde_json::from_str(&record.import.stages_json).unwrap_or_default(),
        diagnostics: serde_json::from_str(&record.import.diagnostics_json).unwrap_or_default(),
        candidates: record
            .candidates
            .into_iter()
            .map(|candidate| ModelImportCandidateView {
                id: candidate.candidate_id,
                kind: candidate.candidate_kind,
                display_name: candidate.display_name,
                table_name: candidate.table_name,
                schema: candidate
                    .schema_json
                    .and_then(|value| serde_json::from_str(&value).ok()),
                row_count: candidate.row_count,
                selected: candidate.selected,
            })
            .collect(),
        error_code: record.import.error_code,
        error_message: record.import.error_message,
    }
}
