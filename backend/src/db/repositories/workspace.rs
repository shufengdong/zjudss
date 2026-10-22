use diesel::prelude::*;

use crate::auth::AccessScope;
use crate::db::models::{
    DataframeBinding, ModelVersion, NewDataframeBinding, NewModelVersion, NewVersionArtifact,
    NewWorkspace, VersionArtifact, Workspace,
};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Debug, Clone)]
pub struct BindingDraft {
    pub table_kind: Option<String>,
    pub confidence: Option<f64>,
    pub resolution: String,
    pub columns_json: String,
    pub evidence_json: String,
}

#[derive(Debug, Clone)]
pub struct VersionArtifactDraft {
    pub artifact_id: i32,
    pub sha256: String,
    pub role: String,
    pub relative_path: Option<String>,
    pub display_name: String,
    pub source_artifact_id: Option<i32>,
    pub source_artifact_sha256: Option<String>,
    pub sheet_name: Option<String>,
    pub binding: Option<BindingDraft>,
}

#[derive(Debug, Clone)]
pub struct PreparedVersion {
    pub status: String,
    pub source_kind: String,
    pub origin_kind: String,
    pub manifest_json: String,
    pub patch_json: Option<String>,
    pub content_hash: String,
    pub artifacts: Vec<VersionArtifactDraft>,
    pub created_at_ms: i64,
}

#[derive(Debug, Clone)]
pub struct WorkspaceRecord {
    pub workspace: Workspace,
    pub active: Option<ModelVersion>,
    pub draft: Option<ModelVersion>,
}

#[derive(Clone)]
pub struct WorkspaceRepo {
    pool: DbPool,
}

impl WorkspaceRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn get_or_create(
        &self,
        scope: &AccessScope,
        session_id: i32,
        now_ms: i64,
    ) -> RepoResult<WorkspaceRecord> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<WorkspaceRecord> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<WorkspaceRecord, RepoError, _>(|conn| {
                ensure_session(conn, &scope, session_id)?;
                diesel::insert_into(schema::workspaces::table)
                    .values(NewWorkspace {
                        session_id,
                        created_at_ms: now_ms,
                        updated_at_ms: now_ms,
                    })
                    .on_conflict(schema::workspaces::session_id)
                    .do_nothing()
                    .execute(conn)?;
                let workspace = schema::workspaces::table
                    .filter(schema::workspaces::session_id.eq(session_id))
                    .first::<Workspace>(conn)?;
                load_record(conn, workspace)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find_by_session(
        &self,
        scope: &AccessScope,
        session_id: i32,
    ) -> RepoResult<WorkspaceRecord> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<WorkspaceRecord> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            ensure_session(&mut conn, &scope, session_id)?;
            let workspace = schema::workspaces::table
                .filter(schema::workspaces::session_id.eq(session_id))
                .first::<Workspace>(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            load_record(&mut conn, workspace)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn publish(
        &self,
        scope: &AccessScope,
        workspace_id: i32,
        expected_base: Option<i32>,
        version: PreparedVersion,
    ) -> RepoResult<(ModelVersion, WorkspaceRecord)> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<(ModelVersion, WorkspaceRecord)> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<(ModelVersion, WorkspaceRecord), RepoError, _>(|conn| {
                let workspace = scoped_workspace(conn, &scope, workspace_id)?;
                let storage_scope_id = schema::workspaces::table
                    .inner_join(schema::sessions::table)
                    .filter(schema::workspaces::id.eq(workspace_id))
                    .select(schema::sessions::storage_scope_id)
                    .first::<Option<i32>>(conn)?
                    .ok_or_else(|| RepoError::InvalidState("会话缺少存储 scope".to_string()))?;
                let current_base = workspace.draft_version_id.or(workspace.active_version_id);
                if current_base != expected_base {
                    return Err(RepoError::VersionConflict);
                }

                diesel::insert_into(schema::model_versions::table)
                    .values(NewModelVersion {
                        workspace_id,
                        parent_version_id: current_base,
                        status: version.status.clone(),
                        source_kind: version.source_kind,
                        origin_kind: version.origin_kind,
                        manifest_json: version.manifest_json,
                        patch_json: version.patch_json,
                        content_hash: version.content_hash,
                        created_at_ms: version.created_at_ms,
                    })
                    .execute(conn)
                    .map_err(|error| match error {
                        diesel::result::Error::DatabaseError(
                            diesel::result::DatabaseErrorKind::UniqueViolation,
                            _,
                        ) => RepoError::VersionConflict,
                        other => RepoError::Query(other),
                    })?;
                let version_id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(conn)?;
                let model = schema::model_versions::table
                    .find(version_id)
                    .first::<ModelVersion>(conn)?;
                for input in version.artifacts {
                    ensure_artifact_owned(
                        conn,
                        storage_scope_id,
                        input.artifact_id,
                        &input.sha256,
                    )?;
                    if let Some(source_id) = input.source_artifact_id {
                        ensure_artifact_owned(
                            conn,
                            storage_scope_id,
                            source_id,
                            input.source_artifact_sha256.as_deref().unwrap_or(""),
                        )?;
                    }
                    diesel::insert_into(schema::version_artifacts::table)
                        .values(NewVersionArtifact {
                            version_id,
                            artifact_id: input.artifact_id,
                            role: input.role,
                            relative_path: input.relative_path,
                            display_name: input.display_name,
                            source_artifact_id: input.source_artifact_id,
                            sheet_name: input.sheet_name,
                        })
                        .execute(conn)?;
                    if let Some(binding) = input.binding {
                        let input_id: i32 = diesel::dsl::sql::<diesel::sql_types::Integer>(
                            "SELECT last_insert_rowid()",
                        )
                        .get_result(conn)?;
                        diesel::insert_into(schema::dataframe_bindings::table)
                            .values(NewDataframeBinding {
                                version_artifact_id: input_id,
                                table_kind: binding.table_kind,
                                confidence: binding.confidence,
                                resolution: binding.resolution,
                                columns_json: binding.columns_json,
                                evidence_json: binding.evidence_json,
                            })
                            .execute(conn)?;
                    }
                }
                if workspace.active_version_id.is_none() && model.status == "ready" {
                    diesel::update(schema::workspaces::table.find(workspace_id))
                        .set((
                            schema::workspaces::active_version_id.eq(Some(version_id)),
                            schema::workspaces::draft_version_id.eq::<Option<i32>>(None),
                            schema::workspaces::updated_at_ms.eq(version.created_at_ms),
                        ))
                        .execute(conn)?;
                } else {
                    diesel::update(schema::workspaces::table.find(workspace_id))
                        .set((
                            schema::workspaces::draft_version_id.eq(Some(version_id)),
                            schema::workspaces::updated_at_ms.eq(version.created_at_ms),
                        ))
                        .execute(conn)?;
                }
                let workspace = schema::workspaces::table
                    .find(workspace_id)
                    .first::<Workspace>(conn)?;
                Ok((model, load_record(conn, workspace)?))
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn activate(
        &self,
        scope: &AccessScope,
        workspace_id: i32,
        version_id: i32,
        now_ms: i64,
    ) -> RepoResult<WorkspaceRecord> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<WorkspaceRecord> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<WorkspaceRecord, RepoError, _>(|conn| {
                let workspace = scoped_workspace(conn, &scope, workspace_id)?;
                let version = schema::model_versions::table
                    .find(version_id)
                    .first::<ModelVersion>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if version.workspace_id != workspace_id {
                    return Err(RepoError::NotFound);
                }
                if version.status != "ready" || workspace.draft_version_id != Some(version_id) {
                    return Err(RepoError::InvalidState(
                        "只能激活当前 ready draft".to_string(),
                    ));
                }
                diesel::update(schema::workspaces::table.find(workspace_id))
                    .set((
                        schema::workspaces::active_version_id.eq(Some(version_id)),
                        schema::workspaces::draft_version_id.eq::<Option<i32>>(None),
                        schema::workspaces::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                let workspace = schema::workspaces::table
                    .find(workspace_id)
                    .first::<Workspace>(conn)?;
                load_record(conn, workspace)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn activate_checked(
        &self,
        scope: &AccessScope,
        workspace_id: i32,
        version_id: i32,
        expected_base: Option<i32>,
        now_ms: i64,
    ) -> RepoResult<WorkspaceRecord> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<WorkspaceRecord> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<WorkspaceRecord, RepoError, _>(|conn| {
                let workspace = scoped_workspace(conn, &scope, workspace_id)?;
                if workspace.draft_version_id.or(workspace.active_version_id) != expected_base {
                    return Err(RepoError::VersionConflict);
                }
                let version = schema::model_versions::table
                    .find(version_id)
                    .first::<ModelVersion>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if version.workspace_id != workspace_id {
                    return Err(RepoError::NotFound);
                }
                if version.status != "ready" || workspace.draft_version_id != Some(version_id) {
                    return Err(RepoError::InvalidState("只能激活当前 ready draft".to_string()));
                }
                diesel::update(schema::workspaces::table.find(workspace_id))
                    .set((
                        schema::workspaces::active_version_id.eq(Some(version_id)),
                        schema::workspaces::draft_version_id.eq::<Option<i32>>(None),
                        schema::workspaces::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                let workspace = schema::workspaces::table.find(workspace_id).first(conn)?;
                load_record(conn, workspace)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn rollback(
        &self,
        scope: &AccessScope,
        workspace_id: i32,
        version_id: i32,
        now_ms: i64,
    ) -> RepoResult<WorkspaceRecord> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<WorkspaceRecord> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<WorkspaceRecord, RepoError, _>(|conn| {
                let workspace = scoped_workspace(conn, &scope, workspace_id)?;
                let version = schema::model_versions::table
                    .find(version_id)
                    .first::<ModelVersion>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if version.workspace_id != workspace_id {
                    return Err(RepoError::NotFound);
                }
                if version.status != "ready" {
                    return Err(RepoError::InvalidState("只能回退到 ready 版本".to_string()));
                }
                if workspace.draft_version_id == Some(version_id) {
                    return Err(RepoError::InvalidState(
                        "当前 draft 请使用激活操作".to_string(),
                    ));
                }
                diesel::update(schema::workspaces::table.find(workspace_id))
                    .set((
                        schema::workspaces::active_version_id.eq(Some(version_id)),
                        schema::workspaces::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                let workspace = schema::workspaces::table
                    .find(workspace_id)
                    .first::<Workspace>(conn)?;
                load_record(conn, workspace)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn rollback_checked(
        &self,
        scope: &AccessScope,
        workspace_id: i32,
        version_id: i32,
        expected_base: Option<i32>,
        now_ms: i64,
    ) -> RepoResult<WorkspaceRecord> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<WorkspaceRecord> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<WorkspaceRecord, RepoError, _>(|conn| {
                let workspace = scoped_workspace(conn, &scope, workspace_id)?;
                if workspace.draft_version_id.or(workspace.active_version_id) != expected_base {
                    return Err(RepoError::VersionConflict);
                }
                let version = schema::model_versions::table
                    .find(version_id)
                    .first::<ModelVersion>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                if version.workspace_id != workspace_id {
                    return Err(RepoError::NotFound);
                }
                if version.status != "ready" {
                    return Err(RepoError::InvalidState("只能回退到 ready 版本".to_string()));
                }
                if workspace.draft_version_id == Some(version_id) {
                    return Err(RepoError::InvalidState("当前 draft 请使用激活操作".to_string()));
                }
                diesel::update(schema::workspaces::table.find(workspace_id))
                    .set((
                        schema::workspaces::active_version_id.eq(Some(version_id)),
                        schema::workspaces::updated_at_ms.eq(now_ms),
                    ))
                    .execute(conn)?;
                let workspace = schema::workspaces::table.find(workspace_id).first(conn)?;
                load_record(conn, workspace)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_versions(
        &self,
        scope: &AccessScope,
        session_id: i32,
    ) -> RepoResult<Vec<ModelVersion>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<ModelVersion>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            ensure_session(&mut conn, &scope, session_id)?;
            let workspace_id = schema::workspaces::table
                .filter(schema::workspaces::session_id.eq(session_id))
                .select(schema::workspaces::id)
                .first::<i32>(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            schema::model_versions::table
                .filter(schema::model_versions::workspace_id.eq(workspace_id))
                .order((
                    schema::model_versions::created_at_ms.desc(),
                    schema::model_versions::id.desc(),
                ))
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find_version(
        &self,
        scope: &AccessScope,
        version_id: i32,
    ) -> RepoResult<ModelVersion> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<ModelVersion> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::model_versions::table
                .inner_join(schema::workspaces::table.inner_join(schema::sessions::table))
                .filter(schema::model_versions::id.eq(version_id))
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::model_versions::all_columns)
                .first(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn inputs(
        &self,
        scope: &AccessScope,
        version_id: i32,
    ) -> RepoResult<Vec<(VersionArtifact, Option<DataframeBinding>)>> {
        self.find_version(scope, version_id).await?;
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(
            move || -> RepoResult<Vec<(VersionArtifact, Option<DataframeBinding>)>> {
                let mut conn = pool.get().map_err(RepoError::Pool)?;
                let inputs = schema::version_artifacts::table
                    .filter(schema::version_artifacts::version_id.eq(version_id))
                    .order(schema::version_artifacts::id.asc())
                    .load::<VersionArtifact>(&mut conn)?;
                let mut result = Vec::with_capacity(inputs.len());
                for input in inputs {
                    let binding = schema::dataframe_bindings::table
                        .filter(schema::dataframe_bindings::version_artifact_id.eq(input.id))
                        .first::<DataframeBinding>(&mut conn)
                        .optional()?;
                    result.push((input, binding));
                }
                Ok(result)
            },
        )
        .await
        .map_err(RepoError::Join)?
    }
}

fn ensure_session(
    conn: &mut SqliteConnection,
    scope: &AccessScope,
    session_id: i32,
) -> RepoResult<()> {
    schema::sessions::table
        .filter(schema::sessions::id.eq(session_id))
        .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
        .filter(schema::sessions::access_scope_key.eq(&scope.key))
        .select(schema::sessions::id)
        .first::<i32>(conn)
        .optional()?
        .ok_or(RepoError::NotFound)?;
    Ok(())
}

fn scoped_workspace(
    conn: &mut SqliteConnection,
    scope: &AccessScope,
    workspace_id: i32,
) -> RepoResult<Workspace> {
    schema::workspaces::table
        .inner_join(schema::sessions::table)
        .filter(schema::workspaces::id.eq(workspace_id))
        .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
        .filter(schema::sessions::access_scope_key.eq(&scope.key))
        .select(schema::workspaces::all_columns)
        .first(conn)
        .optional()?
        .ok_or(RepoError::NotFound)
}

fn load_record(conn: &mut SqliteConnection, workspace: Workspace) -> RepoResult<WorkspaceRecord> {
    let active = match workspace.active_version_id {
        Some(id) => Some(schema::model_versions::table.find(id).first(conn)?),
        None => None,
    };
    let draft = match workspace.draft_version_id {
        Some(id) => Some(schema::model_versions::table.find(id).first(conn)?),
        None => None,
    };
    Ok(WorkspaceRecord {
        workspace,
        active,
        draft,
    })
}

fn ensure_artifact_owned(
    conn: &mut SqliteConnection,
    storage_scope_id: i32,
    artifact_id: i32,
    expected_sha256: &str,
) -> RepoResult<()> {
    schema::storage_scope_artifacts::table
        .find((storage_scope_id, artifact_id))
        .select(schema::storage_scope_artifacts::artifact_id)
        .first::<i32>(conn)
        .optional()?
        .ok_or(RepoError::NotFound)?;
    if !expected_sha256.is_empty() {
        let actual = schema::artifacts::table
            .find(artifact_id)
            .select(schema::artifacts::sha256)
            .first::<String>(conn)?;
        if actual != expected_sha256 {
            return Err(RepoError::InvalidState(
                "artifact hash 与版本 manifest 不一致".to_string(),
            ));
        }
    }
    Ok(())
}
