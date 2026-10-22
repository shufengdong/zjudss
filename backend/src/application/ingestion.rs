use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::Arc;

use serde::Serialize;
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::auth::Principal;
use crate::catalog::{SchemaCatalog, infer_tables};
use crate::db::models::{IngestionArtifact, NewIngestion};
use crate::db::repositories::{IngestionItemDraft, IngestionRecord, IngestionRepo, RepoError};
use crate::ingestion::{TableSchema, TabularFormat, TabularInspector, normalize_excel_to_parquet};
use crate::storage::StagedArtifact;

use super::{
    ArtifactError, ArtifactMetadata, ArtifactRef, ArtifactService, AttachmentFile,
    DataframeBindingDraft, EventBus, EventWriter, MessageRepo, ModelAmbiguity, ModelVersionStatus,
    OperationKind, OperationRef, RecognizedTable, SourceKind, Stage, StageStatus, UploadStatus,
    VersionDraft, VersionInputDraft, VersionOrigin, WorkspaceError, WorkspaceService, now_ms,
};

const STAGE_KEYS: [(&str, &str); 4] = [
    ("inspect", "检查文件"),
    ("infer", "推断模型"),
    ("validate", "校验输入"),
    ("publish", "发布草稿"),
];

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

struct CommittedUpload {
    client_file_id: String,
    file_name: String,
    relative_path: Option<String>,
    media_type: Option<String>,
    artifact: ArtifactRef,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IngestionItemView {
    pub id: String,
    pub client_file_id: String,
    pub artifact_id: String,
    pub sha256: String,
    pub file_name: String,
    pub relative_path: Option<String>,
    pub media_type: Option<String>,
    pub role: String,
    pub source_artifact_id: Option<String>,
    pub sheet_name: Option<String>,
    pub status: String,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AcceptedBatch {
    pub id: String,
    pub workspace_id: String,
    pub client_batch_id: String,
    pub origin_kind: String,
    pub base_version_id: Option<String>,
    pub published_version_id: Option<String>,
    pub status: String,
    pub stages: Vec<Stage>,
    pub items: Vec<IngestionItemView>,
    pub error_code: Option<String>,
    pub error_message: Option<String>,
}

#[derive(Debug)]
pub enum IngestionError {
    NotFound,
    InvalidUpload(String),
    UploadTooLarge(String),
    IdempotencyConflict,
    Artifact(ArtifactError),
    Repository(RepoError),
    Workspace(WorkspaceError),
}

impl std::fmt::Display for IngestionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "资源不存在"),
            Self::InvalidUpload(message) => write!(f, "{message}"),
            Self::UploadTooLarge(message) => write!(f, "{message}"),
            Self::IdempotencyConflict => write!(f, "clientBatchId 已用于不同内容"),
            Self::Artifact(error) => write!(f, "{error}"),
            Self::Repository(error) => write!(f, "{error}"),
            Self::Workspace(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for IngestionError {}

impl From<ArtifactError> for IngestionError {
    fn from(value: ArtifactError) -> Self {
        Self::Artifact(value)
    }
}
impl From<RepoError> for IngestionError {
    fn from(value: RepoError) -> Self {
        match value {
            RepoError::NotFound => Self::NotFound,
            RepoError::IdempotencyConflict => Self::IdempotencyConflict,
            other => Self::Repository(other),
        }
    }
}
impl From<WorkspaceError> for IngestionError {
    fn from(value: WorkspaceError) -> Self {
        Self::Workspace(value)
    }
}

#[derive(Clone)]
pub struct IngestionService {
    repo: IngestionRepo,
    artifacts: ArtifactService,
    workspaces: WorkspaceService,
    catalog: Arc<SchemaCatalog>,
    messages: MessageRepo,
    events: EventBus,
    workspace_locks: Arc<Mutex<HashMap<i32, Arc<Mutex<()>>>>>,
}

impl IngestionService {
    pub fn new(
        repo: IngestionRepo,
        artifacts: ArtifactService,
        workspaces: WorkspaceService,
        catalog: Arc<SchemaCatalog>,
        messages: MessageRepo,
        events: EventBus,
    ) -> Self {
        Self {
            repo,
            artifacts,
            workspaces,
            catalog,
            messages,
            events,
            workspace_locks: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub async fn accept_batch(
        &self,
        principal: &Principal,
        session_id: i32,
        batch: UploadBatch,
    ) -> Result<AcceptedBatch, IngestionError> {
        if batch.client_batch_id.trim().is_empty() || batch.files.is_empty() {
            return Err(IngestionError::InvalidUpload(
                "clientBatchId 和至少一个文件是必需的".to_string(),
            ));
        }
        let mut ids = std::collections::HashSet::new();
        if batch.files.iter().any(|file| {
            file.client_file_id.trim().is_empty() || !ids.insert(file.client_file_id.clone())
        }) {
            return Err(IngestionError::InvalidUpload(
                "clientFileId 必须非空且在批次内唯一".to_string(),
            ));
        }
        let workspace = self
            .workspaces
            .get_or_create(&principal.access, session_id)
            .await?;
        let expected: BTreeMap<_, _> = batch
            .files
            .iter()
            .map(|file| (file.client_file_id.clone(), file.staged.sha256.clone()))
            .collect();
        if let Some(record) = self
            .repo
            .find_by_batch(
                &principal.access,
                workspace.workspace.id,
                &batch.client_batch_id,
            )
            .await?
        {
            return compare_idempotent(record, &expected);
        }

        let storage_scope = self.artifacts.resolve_scope(&principal.storage).await?;
        let mut committed = Vec::with_capacity(batch.files.len());
        for file in batch.files {
            let format = format_from_name(&file.file_name)
                .unwrap_or("file")
                .to_string();
            let UploadFile {
                client_file_id,
                file_name,
                relative_path,
                media_type,
                staged,
            } = file;
            match self
                .artifacts
                .commit(
                    storage_scope.id,
                    staged,
                    ArtifactMetadata {
                        format,
                        media_type: media_type.clone(),
                    },
                )
                .await
            {
                Ok(artifact) => committed.push(CommittedUpload {
                    client_file_id,
                    file_name,
                    relative_path,
                    media_type,
                    artifact,
                }),
                Err(error) => {
                    release_all(&self.artifacts, storage_scope.id, &committed).await;
                    return Err(error.into());
                }
            }
        }
        let now = now_ms();
        let stages = initial_stages();
        let new_ingestion = NewIngestion {
            workspace_id: workspace.workspace.id,
            client_batch_id: batch.client_batch_id.clone(),
            origin_kind: "upload".to_string(),
            source_ref_json: None,
            base_version_id: workspace
                .workspace
                .draft_version_id
                .or(workspace.workspace.active_version_id),
            published_version_id: None,
            status: "accepted".to_string(),
            stages_json: serde_json::to_string(&stages).expect("serialize stages"),
            error_code: None,
            error_message: None,
            created_at_ms: now,
            updated_at_ms: now,
        };
        let item_drafts = committed
            .iter()
            .map(|file| IngestionItemDraft {
                client_file_id: file.client_file_id.clone(),
                artifact_id: file.artifact.id,
                file_name: file.file_name.clone(),
                relative_path: file.relative_path.clone(),
                media_type: file.media_type.clone(),
            })
            .collect();
        let record = match self
            .repo
            .create(&principal.access, new_ingestion, item_drafts)
            .await
        {
            Ok(record) => record,
            Err(RepoError::IdempotencyConflict) => {
                let existing = self
                    .repo
                    .find_by_batch(
                        &principal.access,
                        workspace.workspace.id,
                        &batch.client_batch_id,
                    )
                    .await?
                    .ok_or(IngestionError::IdempotencyConflict)?;
                release_all(&self.artifacts, storage_scope.id, &committed).await;
                return compare_idempotent(existing, &expected);
            }
            Err(error) => {
                release_all(&self.artifacts, storage_scope.id, &committed).await;
                return Err(error.into());
            }
        };
        let writer = self.writer(principal, session_id);
        for (item, artifact) in &record.items {
            writer
                .attachment(
                    attachment(item, artifact.size_bytes),
                    record.ingestion.client_batch_id.clone(),
                    item.client_file_id.clone(),
                    record.ingestion.id.to_string(),
                    UploadStatus::Accepted,
                )
                .await?;
        }
        Ok(view(record))
    }

    pub async fn get(
        &self,
        principal: &Principal,
        ingestion_id: i32,
    ) -> Result<AcceptedBatch, IngestionError> {
        Ok(view(self.repo.find(&principal.access, ingestion_id).await?))
    }

    pub async fn active(
        &self,
        principal: &Principal,
        workspace_id: i32,
    ) -> Result<Vec<AcceptedBatch>, IngestionError> {
        Ok(self
            .repo
            .list_active(&principal.access, workspace_id)
            .await?
            .into_iter()
            .map(view)
            .collect())
    }

    pub async fn process(&self, principal: Principal, ingestion_id: i32) {
        if let Err(error) = self.process_inner(&principal, ingestion_id).await {
            let code = match &error {
                IngestionError::Workspace(WorkspaceError::VersionConflict) => "version_conflict",
                _ => "ingestion_failed",
            };
            let stages = failed_stages();
            let _ = self
                .repo
                .set_ingestion_status(
                    ingestion_id,
                    "failed",
                    serde_json::to_string(&stages).unwrap(),
                    None,
                    Some((code.to_string(), error.to_string())),
                    now_ms(),
                )
                .await;
            if let Ok(record) = self.repo.find(&principal.access, ingestion_id).await {
                let _ = self
                    .writer(&principal, record.session_id)
                    .progress(
                        OperationRef {
                            kind: OperationKind::Ingestion,
                            id: ingestion_id.to_string(),
                        },
                        stages,
                    )
                    .await;
            }
        }
    }

    async fn process_inner(
        &self,
        principal: &Principal,
        ingestion_id: i32,
    ) -> Result<(), IngestionError> {
        let first = self.repo.find(&principal.access, ingestion_id).await?;
        if !matches!(
            first.ingestion.status.as_str(),
            "accepted" | "extracting" | "inferring" | "validating"
        ) {
            return Ok(());
        }
        let lock = {
            let mut locks = self.workspace_locks.lock().await;
            locks
                .entry(first.ingestion.workspace_id)
                .or_insert_with(|| Arc::new(Mutex::new(())))
                .clone()
        };
        let _guard = lock.lock().await;
        let record = self.repo.find(&principal.access, ingestion_id).await?;
        if !matches!(
            record.ingestion.status.as_str(),
            "accepted" | "extracting" | "inferring" | "validating"
        ) {
            return Ok(());
        }
        self.set_stage(principal, &record, 0, None).await?;
        let writer = self.writer(principal, record.session_id);
        for (item, artifact) in record
            .items
            .iter()
            .filter(|(item, _)| item.role == "uploaded")
        {
            self.repo
                .set_item_status(item.id, "processing", None, now_ms())
                .await?;
            writer
                .attachment(
                    attachment(item, artifact.size_bytes),
                    record.ingestion.client_batch_id.clone(),
                    item.client_file_id.clone(),
                    ingestion_id.to_string(),
                    UploadStatus::Ingesting,
                )
                .await?;
        }

        let scope = self.artifacts.resolve_scope(&principal.storage).await?;
        let temp = self
            .artifacts
            .store()
            .data_dir()
            .join("ingestion-work")
            .join(ingestion_id.to_string());
        std::fs::create_dir_all(&temp)
            .map_err(|error| IngestionError::InvalidUpload(error.to_string()))?;
        let mut prepared: Vec<(
            IngestionArtifact,
            ArtifactRef,
            TableSchema,
            Option<(i32, String)>,
        )> = Vec::new();
        let mut failed = 0usize;
        for (item, artifact) in record
            .items
            .iter()
            .filter(|(item, _)| item.role == "uploaded")
        {
            let path = self
                .artifacts
                .store()
                .resolve(&artifact.storage_path)
                .map_err(ArtifactError::from)?;
            let result: Result<
                Vec<(
                    IngestionArtifact,
                    ArtifactRef,
                    TableSchema,
                    Option<(i32, String)>,
                )>,
                String,
            > = match tabular_format(&item.file_name) {
                Some(TabularFormat::Csv) | Some(TabularFormat::Parquet) => {
                    let format = tabular_format(&item.file_name).unwrap();
                    TabularInspector::inspect(&path, format)
                        .map_err(|e| e.to_string())
                        .and_then(|tables| {
                            if tables.len() != 1 {
                                Err("文件不包含可读取的数据表".to_string())
                            } else {
                                Ok(vec![(
                                    item.clone(),
                                    ArtifactRef::from(artifact.clone()),
                                    tables[0].clone(),
                                    None,
                                )])
                            }
                        })
                }
                Some(TabularFormat::Xls) | Some(TabularFormat::Xlsx) => {
                    match normalize_excel_to_parquet(&path, &temp.join(format!("item-{}", item.id)))
                    {
                        Ok(sheets) if sheets.is_empty() => Err("Excel 没有非空工作表".to_string()),
                        Ok(sheets) => {
                            let mut values = Vec::new();
                            let mut normalization_error = None;
                            for sheet in sheets {
                                let parquet_file = match std::fs::File::open(&sheet.path) {
                                    Ok(file) => file,
                                    Err(error) => {
                                        normalization_error = Some(error.to_string());
                                        break;
                                    }
                                };
                                let staged = match self
                                    .artifacts
                                    .store()
                                    .stage(parquet_file, crate::storage::StageLimits::default())
                                {
                                    Ok(value) => value,
                                    Err(e) => {
                                        normalization_error = Some(e.to_string());
                                        break;
                                    }
                                };
                                match self
                                    .artifacts
                                    .commit(
                                        scope.id,
                                        staged,
                                        ArtifactMetadata {
                                            format: "parquet".into(),
                                            media_type: Some(
                                                "application/vnd.apache.parquet".into(),
                                            ),
                                        },
                                    )
                                    .await
                                {
                                    Ok(normalized) => {
                                        let mut normalized_item = match self
                                            .repo
                                            .add_normalized(
                                                ingestion_id,
                                                item,
                                                normalized.id,
                                                format!(
                                                    "{}-{}.parquet",
                                                    item.file_name, sheet.sheet_name
                                                ),
                                                sheet.sheet_name.clone(),
                                                now_ms(),
                                            )
                                            .await
                                        {
                                            Ok(value) => value,
                                            Err(error) => {
                                                normalization_error = Some(error.to_string());
                                                break;
                                            }
                                        };
                                        normalized_item.relative_path = Some(format!(
                                            "normalized/{}/sheet-{}.parquet",
                                            item.client_file_id,
                                            values.len(),
                                        ));
                                        values.push((
                                            normalized_item,
                                            normalized,
                                            sheet.schema,
                                            Some((artifact.id, artifact.sha256.clone())),
                                        ));
                                    }
                                    Err(e) => {
                                        normalization_error = Some(e.to_string());
                                        break;
                                    }
                                }
                            }
                            normalization_error.map_or(Ok(values), Err)
                        }
                        Err(error) => Err(error.to_string()),
                    }
                }
                None => Err("仅支持 CSV、Excel 和 Parquet DataFrame 文件".to_string()),
            };
            match result {
                Ok(mut values) => {
                    prepared.append(&mut values);
                    self.repo
                        .set_item_status(item.id, "ready", None, now_ms())
                        .await?;
                    writer
                        .attachment(
                            attachment(item, artifact.size_bytes),
                            record.ingestion.client_batch_id.clone(),
                            item.client_file_id.clone(),
                            ingestion_id.to_string(),
                            UploadStatus::Ready,
                        )
                        .await?;
                }
                Err(message) => {
                    failed += 1;
                    self.repo
                        .set_item_status(
                            item.id,
                            "failed",
                            Some(("unsupported_or_invalid_table".into(), message)),
                            now_ms(),
                        )
                        .await?;
                    writer
                        .attachment(
                            attachment(item, artifact.size_bytes),
                            record.ingestion.client_batch_id.clone(),
                            item.client_file_id.clone(),
                            ingestion_id.to_string(),
                            UploadStatus::Failed,
                        )
                        .await?;
                }
            }
        }
        if prepared.is_empty() {
            let stages = failed_stages();
            self.repo
                .set_ingestion_status(
                    ingestion_id,
                    "failed",
                    serde_json::to_string(&stages).unwrap(),
                    None,
                    Some((
                        "no_usable_input".into(),
                        "没有可用于发布模型版本的输入".into(),
                    )),
                    now_ms(),
                )
                .await?;
            writer
                .progress(
                    OperationRef {
                        kind: OperationKind::Ingestion,
                        id: ingestion_id.to_string(),
                    },
                    stages,
                )
                .await?;
            return Ok(());
        }
        self.set_stage(principal, &record, 1, None).await?;
        let schemas: Vec<_> = prepared
            .iter()
            .map(|(_, _, schema, _)| schema.clone())
            .collect();
        let evidence = infer_tables(&schemas, &self.catalog);
        self.set_stage(principal, &record, 2, None).await?;

        let mut inputs = base_inputs(
            &self.workspaces,
            &principal.access,
            record.ingestion.base_version_id,
        )
        .await?;
        let mut recognized = Vec::new();
        let mut ambiguities = Vec::new();
        for ((item, artifact, schema, source), inference) in prepared.into_iter().zip(evidence) {
            let evidence_json = serde_json::to_value(&inference).unwrap();
            let input_id = format!(
                "{}:{}",
                item.client_file_id,
                item.sheet_name.clone().unwrap_or_else(|| "table".into())
            );
            if let Some(kind) = &inference.table_kind {
                recognized.push(RecognizedTable {
                    input_id: input_id.clone(),
                    artifact_id: artifact.id.to_string(),
                    table_kind: kind.clone(),
                    confidence: inference.confidence,
                });
            } else {
                ambiguities.push(ModelAmbiguity {
                    input_id: input_id.clone(),
                    candidates: inference
                        .candidates
                        .iter()
                        .take(3)
                        .map(|c| c.table_kind.clone())
                        .collect(),
                });
            }
            inputs.push(VersionInputDraft {
                artifact_id: artifact.id,
                sha256: artifact.sha256,
                role: "input".into(),
                relative_path: item.relative_path.clone(),
                display_name: item.file_name,
                source_artifact_id: source.as_ref().map(|value| value.0),
                source_artifact_sha256: source.map(|value| value.1),
                sheet_name: item.sheet_name,
                binding: Some(DataframeBindingDraft {
                    table_kind: inference.table_kind,
                    confidence: Some(inference.confidence),
                    resolution: inference.resolution,
                    columns: serde_json::to_value(
                        &schema
                            .columns
                            .iter()
                            .map(|c| json!({"name":c.name,"dtype":c.dtype}))
                            .collect::<Vec<_>>(),
                    )
                    .unwrap(),
                    evidence: evidence_json,
                }),
            });
        }
        let status = if failed == 0 && !inputs.is_empty() && ambiguities.is_empty() {
            ModelVersionStatus::Ready
        } else {
            ModelVersionStatus::Collecting
        };
        self.set_stage(principal, &record, 3, None).await?;
        let version = self
            .workspaces
            .publish_version(
                &principal.access,
                record.ingestion.workspace_id,
                record.ingestion.base_version_id,
                VersionDraft {
                    status,
                    source_kind: SourceKind::Dataframes,
                    origin: VersionOrigin::Upload,
                    manifest: json!({
                        "ingestionId": ingestion_id.to_string(),
                        "recognizedTables": recognized,
                        "missing": [],
                        "ambiguities": ambiguities,
                    }),
                    patch: None,
                    inputs,
                },
            )
            .await?;
        let final_status = if failed > 0 {
            "completed_with_errors"
        } else {
            "succeeded"
        };
        let stages = done_stages();
        self.repo
            .set_ingestion_status(
                ingestion_id,
                final_status,
                serde_json::to_string(&stages).unwrap(),
                Some(version.id),
                None,
                now_ms(),
            )
            .await?;
        writer
            .progress(
                OperationRef {
                    kind: OperationKind::Ingestion,
                    id: ingestion_id.to_string(),
                },
                stages,
            )
            .await?;
        let _ = std::fs::remove_dir_all(&temp);
        Ok(())
    }

    async fn set_stage(
        &self,
        principal: &Principal,
        record: &IngestionRecord,
        current: usize,
        error: Option<(String, String)>,
    ) -> Result<(), IngestionError> {
        let stages = running_stages(current);
        let status = match current {
            0 => "extracting",
            1 => "inferring",
            2 | 3 => "validating",
            _ => "validating",
        };
        self.repo
            .set_ingestion_status(
                record.ingestion.id,
                status,
                serde_json::to_string(&stages).unwrap(),
                None,
                error,
                now_ms(),
            )
            .await?;
        self.writer(principal, record.session_id)
            .progress(
                OperationRef {
                    kind: OperationKind::Ingestion,
                    id: record.ingestion.id.to_string(),
                },
                stages,
            )
            .await?;
        Ok(())
    }

    fn writer(&self, principal: &Principal, session_id: i32) -> EventWriter {
        EventWriter::new(
            principal.access.clone(),
            session_id,
            self.messages.clone(),
            self.events.clone(),
        )
    }
}

async fn base_inputs(
    workspaces: &WorkspaceService,
    scope: &crate::auth::AccessScope,
    version_id: Option<i32>,
) -> Result<Vec<VersionInputDraft>, IngestionError> {
    let Some(version_id) = version_id else {
        return Ok(Vec::new());
    };
    let version = workspaces.find_version(scope, version_id).await?;
    let manifest: Value = serde_json::from_str(&version.manifest_json).unwrap_or_default();
    let manifest_inputs = manifest
        .get("inputs")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let rows = workspaces.inputs(scope, version_id).await?;
    Ok(rows
        .into_iter()
        .zip(manifest_inputs)
        .map(|((input, binding), json)| VersionInputDraft {
            artifact_id: input.artifact_id,
            sha256: json
                .get("artifactSha256")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            role: input.role,
            relative_path: input.relative_path,
            display_name: input.display_name,
            source_artifact_id: input.source_artifact_id,
            source_artifact_sha256: json
                .get("sourceArtifactSha256")
                .and_then(Value::as_str)
                .map(str::to_string),
            sheet_name: input.sheet_name,
            binding: binding.map(|binding| DataframeBindingDraft {
                table_kind: binding.table_kind,
                confidence: binding.confidence,
                resolution: binding.resolution,
                columns: serde_json::from_str(&binding.columns_json).unwrap_or_default(),
                evidence: serde_json::from_str(&binding.evidence_json).unwrap_or_default(),
            }),
        })
        .collect())
}

async fn release_all(artifacts: &ArtifactService, scope_id: i32, committed: &[CommittedUpload]) {
    for file in committed {
        let _ = artifacts.release(scope_id, file.artifact.id).await;
    }
}

fn compare_idempotent(
    record: IngestionRecord,
    expected: &BTreeMap<String, String>,
) -> Result<AcceptedBatch, IngestionError> {
    let actual: BTreeMap<_, _> = record
        .items
        .iter()
        .filter(|(item, _)| item.role == "uploaded")
        .map(|(item, artifact)| (item.client_file_id.clone(), artifact.sha256.clone()))
        .collect();
    if &actual == expected {
        Ok(view(record))
    } else {
        Err(IngestionError::IdempotencyConflict)
    }
}

fn view(record: IngestionRecord) -> AcceptedBatch {
    AcceptedBatch {
        id: record.ingestion.id.to_string(),
        workspace_id: record.ingestion.workspace_id.to_string(),
        client_batch_id: record.ingestion.client_batch_id,
        origin_kind: record.ingestion.origin_kind,
        base_version_id: record.ingestion.base_version_id.map(|id| id.to_string()),
        published_version_id: record
            .ingestion
            .published_version_id
            .map(|id| id.to_string()),
        status: record.ingestion.status,
        stages: serde_json::from_str(&record.ingestion.stages_json).unwrap_or_default(),
        items: record
            .items
            .into_iter()
            .map(|(item, artifact)| IngestionItemView {
                id: item.id.to_string(),
                client_file_id: item.client_file_id,
                artifact_id: artifact.id.to_string(),
                sha256: artifact.sha256,
                file_name: item.file_name,
                relative_path: item.relative_path,
                media_type: item.media_type,
                role: item.role,
                source_artifact_id: item.source_artifact_id.map(|id| id.to_string()),
                sheet_name: item.sheet_name,
                status: item.status,
                error_code: item.error_code,
                error_message: item.error_message,
            })
            .collect(),
        error_code: record.ingestion.error_code,
        error_message: record.ingestion.error_message,
    }
}

fn attachment(item: &IngestionArtifact, size: i64) -> AttachmentFile {
    AttachmentFile {
        file_name: item.file_name.clone(),
        file_size: human_size(size),
        file_type: item
            .media_type
            .clone()
            .or_else(|| format_from_name(&item.file_name).map(str::to_string))
            .unwrap_or_else(|| "file".into()),
    }
}
fn human_size(bytes: i64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / 1024.0 / 1024.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
fn format_from_name(name: &str) -> Option<&str> {
    Path::new(name)
        .extension()?
        .to_str()
        .map(|v| v.to_ascii_lowercase())
        .and_then(|v| match v.as_str() {
            "csv" => Some("csv"),
            "xls" => Some("xls"),
            "xlsx" => Some("xlsx"),
            "parquet" => Some("parquet"),
            _ => None,
        })
}
fn tabular_format(name: &str) -> Option<TabularFormat> {
    match format_from_name(name)? {
        "csv" => Some(TabularFormat::Csv),
        "xls" => Some(TabularFormat::Xls),
        "xlsx" => Some(TabularFormat::Xlsx),
        "parquet" => Some(TabularFormat::Parquet),
        _ => None,
    }
}
fn initial_stages() -> Vec<Stage> {
    STAGE_KEYS
        .iter()
        .map(|(key, label)| Stage {
            key: (*key).into(),
            label: (*label).into(),
            status: StageStatus::Pending,
        })
        .collect()
}
fn running_stages(current: usize) -> Vec<Stage> {
    STAGE_KEYS
        .iter()
        .enumerate()
        .map(|(index, (key, label))| Stage {
            key: (*key).into(),
            label: (*label).into(),
            status: if index < current {
                StageStatus::Done
            } else if index == current {
                StageStatus::Running
            } else {
                StageStatus::Pending
            },
        })
        .collect()
}
fn done_stages() -> Vec<Stage> {
    STAGE_KEYS
        .iter()
        .map(|(key, label)| Stage {
            key: (*key).into(),
            label: (*label).into(),
            status: StageStatus::Done,
        })
        .collect()
}
fn failed_stages() -> Vec<Stage> {
    STAGE_KEYS
        .iter()
        .enumerate()
        .map(|(index, (key, label))| Stage {
            key: (*key).into(),
            label: (*label).into(),
            status: if index == 3 {
                StageStatus::Failed
            } else {
                StageStatus::Done
            },
        })
        .collect()
}
