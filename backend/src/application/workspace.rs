use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::path::{Component, Path};

use crate::application::{
    EventBus, EventWriter, ModelAmbiguity, ModelScope, ModelVersionSnapshot, ModelVersionStatus,
    RecognizedTable, now_ms,
};
use crate::auth::AccessScope;
use crate::db::models::{DataframeBinding, ModelVersion, VersionArtifact};
use crate::db::repositories::{
    BindingDraft, MessageRepo, PreparedVersion, RepoError, VersionArtifactDraft, WorkspaceRecord,
    WorkspaceRepo,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Rss,
    Rssbundle,
    Dataframes,
    Mixed,
}

impl SourceKind {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Rss => "rss",
            Self::Rssbundle => "rssbundle",
            Self::Dataframes => "dataframes",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone)]
pub enum VersionOrigin {
    Upload,
    Example {
        example_id: String,
        engine_resource_snapshot: String,
        source_tree_hash: String,
    },
    Generated,
}

impl VersionOrigin {
    fn kind(&self) -> &'static str {
        match self {
            Self::Upload => "upload",
            Self::Example { .. } => "example",
            Self::Generated => "generated",
        }
    }

    fn reference(&self) -> Option<Value> {
        match self {
            Self::Example {
                example_id,
                engine_resource_snapshot,
                source_tree_hash,
            } => Some(serde_json::json!({
                "exampleId": example_id,
                "engineResourceSnapshot": engine_resource_snapshot,
                "sourceTreeHash": source_tree_hash,
            })),
            Self::Upload | Self::Generated => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct DataframeBindingDraft {
    pub table_kind: Option<String>,
    pub confidence: Option<f64>,
    pub resolution: String,
    pub columns: Value,
    pub evidence: Value,
}

#[derive(Debug, Clone)]
pub struct VersionInputDraft {
    pub artifact_id: i32,
    pub sha256: String,
    pub role: String,
    pub relative_path: Option<String>,
    pub display_name: String,
    pub source_artifact_id: Option<i32>,
    pub source_artifact_sha256: Option<String>,
    pub sheet_name: Option<String>,
    pub binding: Option<DataframeBindingDraft>,
}

#[derive(Debug, Clone)]
pub struct VersionDraft {
    pub status: ModelVersionStatus,
    pub source_kind: SourceKind,
    pub origin: VersionOrigin,
    pub manifest: Value,
    pub patch: Option<Value>,
    pub inputs: Vec<VersionInputDraft>,
}

#[derive(Debug)]
pub enum WorkspaceError {
    NotFound,
    VersionConflict,
    InvalidState(String),
    InvalidManifest,
    Repository(RepoError),
}

impl std::fmt::Display for WorkspaceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(f, "资源不存在"),
            Self::VersionConflict => write!(f, "模型版本基线已变化"),
            Self::InvalidState(error) => write!(f, "{error}"),
            Self::InvalidManifest => write!(f, "版本 manifest 必须是 JSON 对象"),
            Self::Repository(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for WorkspaceError {}

impl From<RepoError> for WorkspaceError {
    fn from(value: RepoError) -> Self {
        match value {
            RepoError::NotFound => Self::NotFound,
            RepoError::VersionConflict => Self::VersionConflict,
            RepoError::InvalidState(message) => Self::InvalidState(message),
            other => Self::Repository(other),
        }
    }
}

#[derive(Clone)]
pub struct WorkspaceService {
    repo: WorkspaceRepo,
    message_repo: MessageRepo,
    events: EventBus,
}

impl WorkspaceService {
    pub fn new(repo: WorkspaceRepo, message_repo: MessageRepo, events: EventBus) -> Self {
        Self {
            repo,
            message_repo,
            events,
        }
    }

    pub async fn get_or_create(
        &self,
        scope: &AccessScope,
        session_id: i32,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        self.repo
            .get_or_create(scope, session_id, now_ms())
            .await
            .map_err(Into::into)
    }

    pub async fn get(
        &self,
        scope: &AccessScope,
        session_id: i32,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        self.repo
            .find_by_session(scope, session_id)
            .await
            .map_err(Into::into)
    }

    pub async fn publish_version(
        &self,
        scope: &AccessScope,
        workspace_id: i32,
        expected_base: Option<i32>,
        draft: VersionDraft,
    ) -> Result<ModelVersion, WorkspaceError> {
        let prepared = prepare(draft)?;
        let (version, record) = self
            .repo
            .publish(scope, workspace_id, expected_base, prepared)
            .await?;
        self.emit_status(scope, &record).await?;
        Ok(version)
    }

    pub async fn activate(
        &self,
        scope: &AccessScope,
        workspace_id: i32,
        version_id: i32,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        let record = self
            .repo
            .activate(scope, workspace_id, version_id, now_ms())
            .await?;
        self.emit_status(scope, &record).await?;
        Ok(record)
    }

    pub async fn rollback(
        &self,
        scope: &AccessScope,
        workspace_id: i32,
        version_id: i32,
    ) -> Result<WorkspaceRecord, WorkspaceError> {
        let record = self
            .repo
            .rollback(scope, workspace_id, version_id, now_ms())
            .await?;
        self.emit_status(scope, &record).await?;
        Ok(record)
    }

    pub async fn list_versions(
        &self,
        scope: &AccessScope,
        session_id: i32,
    ) -> Result<Vec<ModelVersion>, WorkspaceError> {
        self.repo
            .list_versions(scope, session_id)
            .await
            .map_err(Into::into)
    }

    pub async fn find_version(
        &self,
        scope: &AccessScope,
        version_id: i32,
    ) -> Result<ModelVersion, WorkspaceError> {
        self.repo
            .find_version(scope, version_id)
            .await
            .map_err(Into::into)
    }

    pub async fn inputs(
        &self,
        scope: &AccessScope,
        version_id: i32,
    ) -> Result<Vec<(VersionArtifact, Option<DataframeBinding>)>, WorkspaceError> {
        self.repo
            .inputs(scope, version_id)
            .await
            .map_err(Into::into)
    }

    async fn emit_status(
        &self,
        scope: &AccessScope,
        record: &WorkspaceRecord,
    ) -> Result<(), WorkspaceError> {
        let writer = EventWriter::new(
            scope.clone(),
            record.workspace.session_id,
            self.message_repo.clone(),
            self.events.clone(),
        );
        writer
            .model_status(
                record.workspace.id,
                ModelScope::Active,
                snapshot(record.active.as_ref()),
            )
            .await?;
        writer
            .model_status(
                record.workspace.id,
                ModelScope::Draft,
                snapshot(record.draft.as_ref()),
            )
            .await?;
        Ok(())
    }
}

fn prepare(draft: VersionDraft) -> Result<PreparedVersion, WorkspaceError> {
    let mut manifest = match draft.manifest {
        Value::Object(map) => map,
        _ => return Err(WorkspaceError::InvalidManifest),
    };
    manifest.insert(
        "originKind".to_string(),
        Value::String(draft.origin.kind().to_string()),
    );
    if let Some(origin) = draft.origin.reference() {
        manifest.insert("origin".to_string(), origin);
    }
    let patch_json = draft.patch.as_ref().map(canonical_json);
    if let Some(patch) = draft.patch.as_ref() {
        manifest.insert("patch".to_string(), patch.clone());
    }
    let mut manifest_inputs = Vec::with_capacity(draft.inputs.len());
    let artifacts = draft
        .inputs
        .into_iter()
        .map(|input| {
            if input.relative_path.as_deref().is_some_and(|path| {
                let path = Path::new(path);
                path.is_absolute()
                    || path
                        .components()
                        .any(|part| !matches!(part, Component::Normal(_)))
            }) {
                return Err(WorkspaceError::InvalidState(
                    "版本输入路径必须是规范相对路径".to_string(),
                ));
            }
            if input.source_artifact_id.is_some() != input.source_artifact_sha256.is_some() {
                return Err(WorkspaceError::InvalidState(
                    "来源 artifact ID 与 hash 必须同时提供".to_string(),
                ));
            }
            manifest_inputs.push(serde_json::json!({
                "artifactSha256": input.sha256,
                "role": input.role,
                "relativePath": input.relative_path,
                "displayName": input.display_name,
                "sourceArtifactSha256": input.source_artifact_sha256,
                "sheetName": input.sheet_name,
                "binding": input.binding.as_ref().map(|binding| serde_json::json!({
                    "tableKind": binding.table_kind,
                    "confidence": binding.confidence,
                    "resolution": binding.resolution,
                    "columns": binding.columns,
                    "evidence": binding.evidence,
                })),
            }));
            Ok(VersionArtifactDraft {
                artifact_id: input.artifact_id,
                sha256: input.sha256,
                role: input.role,
                relative_path: input.relative_path,
                display_name: input.display_name,
                source_artifact_id: input.source_artifact_id,
                source_artifact_sha256: input.source_artifact_sha256,
                sheet_name: input.sheet_name,
                binding: input.binding.map(|binding| BindingDraft {
                    table_kind: binding.table_kind,
                    confidence: binding.confidence,
                    resolution: binding.resolution,
                    columns_json: canonical_json(&binding.columns),
                    evidence_json: canonical_json(&binding.evidence),
                }),
            })
        })
        .collect::<Result<Vec<_>, WorkspaceError>>()?;
    manifest.insert("inputs".to_string(), Value::Array(manifest_inputs));
    let manifest_json = canonical_json(&Value::Object(manifest));
    let content_hash = format!("{:x}", Sha256::digest(manifest_json.as_bytes()));
    Ok(PreparedVersion {
        status: status_str(&draft.status).to_string(),
        source_kind: draft.source_kind.as_str().to_string(),
        origin_kind: draft.origin.kind().to_string(),
        manifest_json,
        patch_json,
        content_hash,
        artifacts,
        created_at_ms: now_ms(),
    })
}

pub fn canonical_json(value: &Value) -> String {
    fn normalize(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let mut keys: Vec<_> = map.keys().collect();
                keys.sort();
                let mut sorted = Map::new();
                for key in keys {
                    sorted.insert(key.clone(), normalize(&map[key]));
                }
                Value::Object(sorted)
            }
            Value::Array(values) => Value::Array(values.iter().map(normalize).collect()),
            other => other.clone(),
        }
    }
    serde_json::to_string(&normalize(value)).expect("JSON value is serializable")
}

fn status_str(status: &ModelVersionStatus) -> &'static str {
    match status {
        ModelVersionStatus::Collecting => "collecting",
        ModelVersionStatus::Ready => "ready",
        ModelVersionStatus::Invalid => "invalid",
    }
}

fn snapshot(version: Option<&ModelVersion>) -> Option<ModelVersionSnapshot> {
    version.map(|version| {
        let manifest: Value = serde_json::from_str(&version.manifest_json).unwrap_or_default();
        let recognized_tables = serde_json::from_value::<Vec<RecognizedTable>>(
            manifest
                .get("recognizedTables")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .unwrap_or_default();
        let missing = serde_json::from_value::<Vec<String>>(
            manifest
                .get("missing")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .unwrap_or_default();
        let ambiguities = serde_json::from_value::<Vec<ModelAmbiguity>>(
            manifest
                .get("ambiguities")
                .cloned()
                .unwrap_or_else(|| serde_json::json!([])),
        )
        .unwrap_or_default();
        ModelVersionSnapshot {
            version_id: version.id.to_string(),
            status: parse_status(&version.status),
            recognized_tables,
            missing,
            ambiguities,
        }
    })
}

fn parse_status(value: &str) -> ModelVersionStatus {
    match value {
        "ready" => ModelVersionStatus::Ready,
        "invalid" => ModelVersionStatus::Invalid,
        _ => ModelVersionStatus::Collecting,
    }
}
