use std::sync::Arc;

use serde_json::Value;

use crate::auth::CurrentUser;
use crate::catalog::{DataFrameContractKind, DataFrameSchemaContract};
use crate::db::repositories::{
    ArtifactDraft, DataResourceRepo, OverlayArtifactSource, RepoError, StructOverlayRequest,
    WorkflowOverlayInput,
};
use crate::engine_resources::EngineResourceSnapshot;
use crate::storage::{ArtifactStore, StageLimits, StagedArtifact};

use super::data_resources::data_resource_parquet::struct_parquet;
use super::{
    ArtifactError, ArtifactMetadata, ArtifactService, DataResourceError, DataResourceService,
    PreparedStructOverlay, now_ms,
};

#[derive(Debug)]
pub enum StructOverlayError {
    Invalid(&'static str, String),
    Artifact(ArtifactError),
    DataResource(DataResourceError),
    Repository(RepoError),
}

impl StructOverlayError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Invalid(code, _) => code,
            Self::Artifact(ArtifactError::NotFound) => "not_found",
            Self::Artifact(ArtifactError::QuotaExceeded) => "storage_quota_exceeded",
            Self::Artifact(_) => "struct_overlay_failed",
            Self::DataResource(error) => error.code(),
            Self::Repository(RepoError::NotFound) => "not_found",
            Self::Repository(_) => "struct_overlay_failed",
        }
    }
}

impl std::fmt::Display for StructOverlayError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(_, message) => write!(formatter, "{message}"),
            Self::Artifact(error) => write!(formatter, "{error}"),
            Self::DataResource(error) => write!(formatter, "{error}"),
            Self::Repository(error) => write!(formatter, "{error}"),
        }
    }
}

impl std::error::Error for StructOverlayError {}

impl From<ArtifactError> for StructOverlayError {
    fn from(value: ArtifactError) -> Self {
        Self::Artifact(value)
    }
}

impl From<DataResourceError> for StructOverlayError {
    fn from(value: DataResourceError) -> Self {
        Self::DataResource(value)
    }
}

impl From<RepoError> for StructOverlayError {
    fn from(value: RepoError) -> Self {
        Self::Repository(value)
    }
}

pub(crate) struct StagedOverlaySet {
    pub inputs: Vec<WorkflowOverlayInput>,
    pub drafts: Vec<ArtifactDraft>,
    pub staged: Vec<StagedArtifact>,
}

impl StagedOverlaySet {
    fn empty() -> Self {
        Self {
            inputs: Vec::new(),
            drafts: Vec::new(),
            staged: Vec::new(),
        }
    }

    pub fn into_parts(
        self,
    ) -> (
        Vec<WorkflowOverlayInput>,
        Vec<ArtifactDraft>,
        Vec<StagedArtifact>,
    ) {
        (self.inputs, self.drafts, self.staged)
    }
}

struct StagedOverlayArtifact {
    artifact: OverlayArtifactSource,
    drafts: Vec<ArtifactDraft>,
    staged: Vec<StagedArtifact>,
}

fn stage_struct_overlay(
    store: &ArtifactStore,
    contract: &DataFrameSchemaContract,
    values: &serde_json::Map<String, Value>,
    reusable_source: Option<(i32, &str)>,
    created_at_ms: i64,
) -> Result<StagedOverlayArtifact, StructOverlayError> {
    let parquet = struct_parquet(contract, values).map_err(StructOverlayError::DataResource)?;
    let staged = store
        .stage(std::io::Cursor::new(parquet), StageLimits::default())
        .map_err(ArtifactError::from)?;
    if let Some((artifact_id, content_hash)) = reusable_source
        && staged.sha256 == content_hash
    {
        return Ok(StagedOverlayArtifact {
            artifact: OverlayArtifactSource::Existing {
                artifact_id,
                content_hash: content_hash.to_string(),
            },
            drafts: Vec::new(),
            staged: Vec::new(),
        });
    }
    let content_hash = staged.sha256.clone();
    let draft = ArtifactMetadata::parquet().into_draft(&staged, created_at_ms)?;
    Ok(StagedOverlayArtifact {
        artifact: OverlayArtifactSource::Batch {
            draft_index: 0,
            content_hash,
        },
        drafts: vec![draft],
        staged: vec![staged],
    })
}

#[derive(Clone)]
pub(crate) struct StructOverlayService {
    resources: DataResourceRepo,
    data_resources: DataResourceService,
    artifacts: ArtifactService,
    snapshot: Arc<EngineResourceSnapshot>,
}

impl StructOverlayService {
    pub fn new(
        resources: DataResourceRepo,
        data_resources: DataResourceService,
        artifacts: ArtifactService,
        snapshot: Arc<EngineResourceSnapshot>,
    ) -> Self {
        Self {
            resources,
            data_resources,
            artifacts,
            snapshot,
        }
    }

    pub async fn stage_initial(
        &self,
        user: &CurrentUser,
        plans: &[PreparedStructOverlay],
    ) -> Result<StagedOverlaySet, StructOverlayError> {
        let mut set = StagedOverlaySet::empty();
        for plan in plans {
            let contract = self.struct_contract(&plan.schema_ref)?;
            let values = plan.effective_values.as_object().ok_or_else(|| {
                Self::invalid("structured_values_invalid", "Struct 参数必须是对象")
            })?;
            let source = if let Some(source_id) = plan.source_resource_id.as_deref() {
                let source = self.resources.find(&user.id, source_id, false).await?;
                let artifact_id = source.resource.normalized_artifact_id.ok_or_else(|| {
                    Self::invalid("configuration_stale", "Struct 来源缺少 artifact")
                })?;
                let schema_ref = source.resource.declared_schema_ref.ok_or_else(|| {
                    Self::invalid("configuration_stale", "Struct 来源缺少 schema")
                })?;
                if source.resource.status != "live" || schema_ref != plan.schema_ref {
                    return Err(Self::invalid(
                        "configuration_stale",
                        "Struct overlay 来源已经变化",
                    ));
                }
                Some((
                    source.resource.id,
                    artifact_id,
                    source.normalized_sha256,
                    source.resource.schema_hash,
                ))
            } else {
                None
            };
            let reusable = source
                .as_ref()
                .map(|(_, artifact_id, hash, _)| (*artifact_id, hash.as_str()));
            let staged =
                stage_struct_overlay(self.artifacts.store(), contract, values, reusable, now_ms())?;
            self.push(
                &mut set,
                plan.input_id.clone(),
                source,
                plan.schema_ref.clone(),
                contract.schema_hash.clone(),
                plan.effective_values.to_string(),
                plan.user_overrides.to_string(),
                plan.selector_overrides.to_string(),
                plan.overrides.to_string(),
                staged,
            );
        }
        Ok(set)
    }

    pub async fn stage_requests(
        &self,
        user: &CurrentUser,
        requests: &[StructOverlayRequest],
    ) -> Result<StagedOverlaySet, StructOverlayError> {
        let mut set = StagedOverlaySet::empty();
        for request in requests {
            let source = self
                .resources
                .find(&user.id, &request.source_resource_id, false)
                .await?;
            if source.resource.normalized_artifact_id != Some(request.source_artifact_id)
                || source.normalized_sha256 != request.source_content_hash
                || source.resource.schema_hash != request.source_schema_hash
                || source.resource.declared_schema_ref.as_deref()
                    != Some(request.schema_ref.as_str())
            {
                return Err(Self::invalid(
                    "configuration_stale",
                    "Struct overlay 来源已经变化",
                ));
            }
            let page = self
                .data_resources
                .rows(user, &request.source_resource_id, Some(1), None, Vec::new())
                .await?;
            let row = page.rows.first().ok_or_else(|| {
                Self::invalid("structured_values_invalid", "Struct 参数缺少数据行")
            })?;
            let mut values = page
                .columns
                .iter()
                .cloned()
                .zip(row.iter().cloned())
                .collect::<serde_json::Map<_, _>>();
            let overrides = request.overrides.as_object().ok_or_else(|| {
                Self::invalid("structured_values_invalid", "Struct 覆盖必须是对象")
            })?;
            for (field, value) in overrides {
                values.insert(field.clone(), value.clone());
            }
            let contract = self.struct_contract(&request.schema_ref)?;
            let staged = stage_struct_overlay(
                self.artifacts.store(),
                contract,
                &values,
                Some((request.source_artifact_id, &request.source_content_hash)),
                now_ms(),
            )?;
            self.push(
                &mut set,
                request.input_id.clone(),
                Some((
                    request.source_resource_id.clone(),
                    request.source_artifact_id,
                    request.source_content_hash.clone(),
                    request.source_schema_hash.clone(),
                )),
                request.schema_ref.clone(),
                contract.schema_hash.clone(),
                Value::Object(values).to_string(),
                request.user_overrides.to_string(),
                request.selector_overrides.to_string(),
                request.overrides.to_string(),
                staged,
            );
        }
        Ok(set)
    }

    #[allow(clippy::too_many_arguments)]
    fn push(
        &self,
        set: &mut StagedOverlaySet,
        input_id: String,
        source: Option<(String, i32, String, String)>,
        schema_ref: String,
        schema_hash: String,
        effective_values_json: String,
        user_overrides_json: String,
        selector_overrides_json: String,
        overrides_json: String,
        mut staged: StagedOverlayArtifact,
    ) {
        if let OverlayArtifactSource::Batch { draft_index, .. } = &mut staged.artifact {
            *draft_index = set.drafts.len();
        }
        set.inputs.push(WorkflowOverlayInput {
            input_id,
            source_resource_id: source.as_ref().map(|value| value.0.clone()),
            source_artifact_id: source.as_ref().map(|value| value.1),
            source_content_hash: source.as_ref().map(|value| value.2.clone()),
            source_schema_hash: source.as_ref().map(|value| value.3.clone()),
            schema_ref,
            schema_hash,
            effective_values_json,
            user_overrides_json,
            selector_overrides_json,
            overrides_json,
            artifact: staged.artifact,
        });
        set.drafts.append(&mut staged.drafts);
        set.staged.append(&mut staged.staged);
    }

    fn struct_contract(
        &self,
        schema_ref: &str,
    ) -> Result<&DataFrameSchemaContract, StructOverlayError> {
        let contract = self
            .snapshot
            .schemas
            .resolve(schema_ref)
            .map_err(|error| Self::invalid("configuration_stale", error.to_string()))?;
        if contract.kind != DataFrameContractKind::Struct {
            return Err(Self::invalid(
                "configuration_stale",
                "研究参数 schema 不再是 Struct",
            ));
        }
        Ok(contract)
    }

    fn invalid(code: &'static str, message: impl Into<String>) -> StructOverlayError {
        StructOverlayError::Invalid(code, message.into())
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::json;

    use crate::engine_resources::EngineResourceSnapshot;
    use crate::storage::ArtifactStore;

    use super::stage_struct_overlay;

    #[test]
    fn staging_overlay_does_not_change_scope_usage_before_repository_commit() {
        let root = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(root.path().to_path_buf()).unwrap();
        let snapshot = EngineResourceSnapshot::minimal(
            "overlay-stage-test",
            "overlay-stage-contract",
            PathBuf::from("overlay-stage-test"),
        );
        let contract = snapshot.schemas.resolve("frames.para").unwrap();
        let values = json!({"max_iter":37}).as_object().unwrap().clone();

        let staged = stage_struct_overlay(&store, contract, &values, None, 1).unwrap();

        assert_eq!(staged.drafts.len(), 1);
        assert_eq!(staged.staged.len(), 1);
        assert!(store.object_paths().unwrap().is_empty());
    }
}
