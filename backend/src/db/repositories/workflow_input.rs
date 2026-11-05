use diesel::prelude::*;
use serde_json::Value;

use crate::auth::UserId;
use crate::db::models::{Artifact, WorkflowInputSnapshotRecord};
use crate::db::{DbPool, schema};

use super::resource_lineage::{
    ResolvedStructResource, frozen_struct_overrides, resolve_live_struct_successor,
};
use super::{RepoError, RepoResult};

#[derive(Debug, Clone)]
pub enum OverlayArtifactSource {
    Existing {
        artifact_id: i32,
        content_hash: String,
    },
    Batch {
        draft_index: usize,
        content_hash: String,
    },
}

#[derive(Debug, Clone)]
pub struct WorkflowOverlayInput {
    pub input_id: String,
    pub source_resource_id: Option<String>,
    pub source_artifact_id: Option<i32>,
    pub source_content_hash: Option<String>,
    pub source_schema_hash: Option<String>,
    pub schema_ref: String,
    pub schema_hash: String,
    pub effective_values_json: String,
    pub user_overrides_json: String,
    pub selector_overrides_json: String,
    pub overrides_json: String,
    pub artifact: OverlayArtifactSource,
}

#[derive(Debug, Clone)]
pub(crate) struct CommittedOverlayInput {
    pub input_id: String,
    pub source_resource_id: Option<String>,
    pub source_artifact_id: Option<i32>,
    pub source_content_hash: Option<String>,
    pub source_schema_hash: Option<String>,
    pub artifact_id: i32,
    pub content_hash: String,
    pub schema_ref: String,
    pub schema_hash: String,
    pub effective_values_json: String,
    pub user_overrides_json: String,
    pub selector_overrides_json: String,
    pub overrides_json: String,
    pub adopts_batch_link: bool,
}

pub(crate) fn commit_overlay_inputs_on(
    conn: &mut SqliteConnection,
    scope_id: i32,
    inputs: Vec<WorkflowOverlayInput>,
    batch: &[Artifact],
) -> RepoResult<Vec<CommittedOverlayInput>> {
    let mut committed = Vec::with_capacity(inputs.len());
    let mut batch_uses = vec![0_u8; batch.len()];
    for input in inputs {
        let (artifact_id, content_hash, adopts_batch_link) = match input.artifact {
            OverlayArtifactSource::Existing {
                artifact_id,
                content_hash,
            } => {
                let owned_hash = schema::artifacts::table
                    .inner_join(schema::storage_scope_artifacts::table)
                    .filter(schema::artifacts::id.eq(artifact_id))
                    .filter(schema::storage_scope_artifacts::storage_scope_id.eq(scope_id))
                    .select(schema::artifacts::sha256)
                    .first::<String>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if owned_hash != content_hash {
                    return Err(RepoError::InvalidState(
                        "overlay_existing_hash_mismatch".into(),
                    ));
                }
                (artifact_id, content_hash, false)
            }
            OverlayArtifactSource::Batch {
                draft_index,
                content_hash,
            } => {
                let uses = batch_uses
                    .get_mut(draft_index)
                    .ok_or_else(|| RepoError::InvalidState("overlay_draft_index_invalid".into()))?;
                *uses = uses.checked_add(1).ok_or_else(|| {
                    RepoError::InvalidState("overlay_draft_reference_overflow".into())
                })?;
                if *uses != 1 {
                    return Err(RepoError::InvalidState(
                        "overlay_draft_reference_duplicate".into(),
                    ));
                }
                let artifact = batch
                    .get(draft_index)
                    .ok_or_else(|| RepoError::InvalidState("overlay_draft_index_invalid".into()))?;
                if artifact.sha256 != content_hash {
                    return Err(RepoError::InvalidState(
                        "overlay_batch_hash_mismatch".into(),
                    ));
                }
                (artifact.id, content_hash, true)
            }
        };
        committed.push(CommittedOverlayInput {
            input_id: input.input_id,
            source_resource_id: input.source_resource_id,
            source_artifact_id: input.source_artifact_id,
            source_content_hash: input.source_content_hash,
            source_schema_hash: input.source_schema_hash,
            artifact_id,
            content_hash,
            schema_ref: input.schema_ref,
            schema_hash: input.schema_hash,
            effective_values_json: input.effective_values_json,
            user_overrides_json: input.user_overrides_json,
            selector_overrides_json: input.selector_overrides_json,
            overrides_json: input.overrides_json,
            adopts_batch_link,
        });
    }
    if batch_uses.iter().any(|uses| *uses != 1) {
        return Err(RepoError::InvalidState(
            "overlay_draft_reference_missing".into(),
        ));
    }
    Ok(committed)
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructOverlayRequest {
    pub input_id: String,
    pub source_resource_id: String,
    pub source_artifact_id: i32,
    pub source_content_hash: String,
    pub source_schema_hash: String,
    pub schema_ref: String,
    pub user_overrides: Value,
    pub selector_overrides: Value,
    pub overrides: Value,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedFrozenStructInput {
    pub(super) request: Option<StructOverlayRequest>,
    pub(super) effective_artifact_id: i32,
    pub(super) effective_content_hash: String,
    pub(super) source_changed: bool,
    pub(super) resource: Option<ResolvedStructResource>,
    pub(super) frozen_resource_id: Option<String>,
    pub(super) frozen_content_hash: Option<String>,
    pub(super) frozen_schema_hash: Option<String>,
    pub(super) snapshot: Value,
}

pub(crate) fn resolve_frozen_struct_input_on(
    conn: &mut SqliteConnection,
    session_id: i32,
    snapshot: &WorkflowInputSnapshotRecord,
) -> RepoResult<ResolvedFrozenStructInput> {
    let frozen_snapshot: Value = serde_json::from_str(&snapshot.snapshot_json)
        .map_err(|error| RepoError::Storage(error.to_string()))?;
    let frozen_source = frozen_snapshot.get("source").unwrap_or(&frozen_snapshot);
    let frozen_resource_id = frozen_source
        .get("resourceId")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| snapshot.resource_id.clone());
    let frozen_content_hash = frozen_source
        .get("contentHash")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| snapshot.content_hash.clone());
    let frozen_schema_hash = frozen_source
        .get("schemaHash")
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| snapshot.schema_hash.clone());

    let Some(frozen_resource_id_value) = frozen_resource_id.as_deref() else {
        if !frozen_snapshot.get("source").is_some_and(Value::is_null)
            && snapshot.resource_id.is_some()
        {
            return Err(RepoError::InvalidState(
                "source_less_snapshot_invalid".into(),
            ));
        }
        return Ok(ResolvedFrozenStructInput {
            request: None,
            effective_artifact_id: snapshot
                .artifact_id
                .ok_or_else(|| RepoError::InvalidState("frozen_artifact_missing".into()))?,
            effective_content_hash: snapshot
                .content_hash
                .clone()
                .ok_or_else(|| RepoError::InvalidState("frozen_content_hash_missing".into()))?,
            source_changed: false,
            resource: None,
            frozen_resource_id,
            frozen_content_hash,
            frozen_schema_hash,
            snapshot: frozen_snapshot,
        });
    };

    let input_alias = snapshot
        .input_alias
        .as_deref()
        .ok_or_else(|| RepoError::InvalidState("frozen_input_alias_missing".into()))?;
    let resource_alias = frozen_source
        .get("resourceAlias")
        .or_else(|| frozen_snapshot.get("resourceAlias"))
        .or_else(|| frozen_snapshot.get("resource_alias"))
        .and_then(Value::as_str)
        .unwrap_or(input_alias);
    let schema_ref = snapshot
        .schema_ref
        .as_deref()
        .ok_or_else(|| RepoError::InvalidState("frozen_schema_ref_missing".into()))?;
    let resource = resolve_live_struct_successor(
        conn,
        session_id,
        frozen_resource_id_value,
        resource_alias,
        schema_ref,
    )?
    .ok_or_else(|| RepoError::InvalidState("struct_successor_unavailable".into()))?;
    let frozen_artifact_id = frozen_source
        .get("artifactId")
        .and_then(Value::as_i64)
        .and_then(|value| i32::try_from(value).ok())
        .or(snapshot.artifact_id);
    let source_changed = frozen_resource_id.as_deref() != Some(resource.id.as_str())
        || frozen_artifact_id != Some(resource.artifact_id)
        || frozen_content_hash.as_deref() != Some(resource.content_hash.as_str())
        || frozen_schema_hash.as_deref() != Some(resource.schema_hash.as_str());
    let request = if source_changed {
        frozen_struct_overrides(&frozen_snapshot)?.map(|overrides| StructOverlayRequest {
            input_id: snapshot.input_name.clone(),
            source_resource_id: resource.id.clone(),
            source_artifact_id: resource.artifact_id,
            source_content_hash: resource.content_hash.clone(),
            source_schema_hash: resource.schema_hash.clone(),
            schema_ref: resource.schema_ref.clone(),
            user_overrides: overrides.user,
            selector_overrides: overrides.selector,
            overrides: overrides.merged,
        })
    } else {
        None
    };
    let (effective_artifact_id, effective_content_hash) = if source_changed {
        (resource.artifact_id, resource.content_hash.clone())
    } else {
        (
            snapshot
                .artifact_id
                .ok_or_else(|| RepoError::InvalidState("frozen_artifact_missing".into()))?,
            snapshot
                .content_hash
                .clone()
                .ok_or_else(|| RepoError::InvalidState("frozen_content_hash_missing".into()))?,
        )
    };
    Ok(ResolvedFrozenStructInput {
        request,
        effective_artifact_id,
        effective_content_hash,
        source_changed,
        resource: Some(resource),
        frozen_resource_id,
        frozen_content_hash,
        frozen_schema_hash,
        snapshot: frozen_snapshot,
    })
}

#[derive(Clone)]
pub struct WorkflowInputRepo {
    pool: DbPool,
}

impl WorkflowInputRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn overlay_requests_for_script(
        &self,
        user: &UserId,
        script_id: i32,
    ) -> Result<Vec<StructOverlayRequest>, RepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let script = schema::script_resources::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::script_resources::session_id)),
                )
                .filter(schema::script_resources::id.eq(script_id))
                .filter(schema::sessions::user_id.eq(&user))
                .filter(schema::sessions::deletion_status.eq("live"))
                .select((
                    schema::script_resources::session_id,
                    schema::script_resources::source_kind,
                    schema::script_resources::managed_workflow_id,
                ))
                .first::<(i32, String, Option<i32>)>(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            if script.1 != "agent" {
                return Ok(Vec::new());
            }
            let Some(workflow_id) = script.2 else {
                return Ok(Vec::new());
            };
            let snapshots = schema::workflow_input_snapshots::table
                .filter(schema::workflow_input_snapshots::workflow_id.eq(workflow_id))
                .filter(schema::workflow_input_snapshots::input_kind.eq("struct"))
                .order(schema::workflow_input_snapshots::input_name.asc())
                .load::<WorkflowInputSnapshotRecord>(&mut conn)?;
            let mut requests = Vec::new();
            for snapshot in snapshots {
                let resolved = resolve_frozen_struct_input_on(&mut conn, script.0, &snapshot)?;
                if let Some(request) = resolved.request {
                    requests.push(request);
                }
            }
            Ok(requests)
        })
        .await
        .map_err(RepoError::Join)?
    }
}
