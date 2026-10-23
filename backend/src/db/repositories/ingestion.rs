use diesel::prelude::*;

use crate::auth::AccessScope;
use crate::db::models::{
    Artifact, Ingestion, IngestionArtifact, NewIngestion, NewIngestionArtifact,
};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Debug, Clone)]
pub struct IngestionItemDraft {
    pub client_file_id: String,
    pub artifact_id: i32,
    pub file_name: String,
    pub relative_path: Option<String>,
    pub media_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IngestionRecord {
    pub ingestion: Ingestion,
    pub items: Vec<(IngestionArtifact, Artifact)>,
    pub session_id: i32,
}

#[derive(Clone)]
pub struct IngestionRepo {
    pool: DbPool,
}

impl IngestionRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn find_by_batch(
        &self,
        scope: &AccessScope,
        workspace_id: i32,
        client_batch_id: &str,
    ) -> RepoResult<Option<IngestionRecord>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        let batch = client_batch_id.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let ingestion = schema::ingestions::table
                .inner_join(schema::workspaces::table.inner_join(schema::sessions::table))
                .filter(schema::ingestions::workspace_id.eq(workspace_id))
                .filter(schema::ingestions::client_batch_id.eq(batch))
                .filter(schema::sessions::user_id.eq(&scope.key))
                .select(schema::ingestions::all_columns)
                .first::<Ingestion>(&mut conn)
                .optional()?;
            ingestion
                .map(|value| load_record(&mut conn, value))
                .transpose()
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn create(
        &self,
        scope: &AccessScope,
        draft: NewIngestion,
        items: Vec<IngestionItemDraft>,
    ) -> RepoResult<IngestionRecord> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<_, RepoError, _>(|conn| {
                ensure_workspace(conn, &scope, draft.workspace_id)?;
                diesel::insert_into(schema::ingestions::table)
                    .values(&draft)
                    .execute(conn)
                    .map_err(|error| match error {
                        diesel::result::Error::DatabaseError(
                            diesel::result::DatabaseErrorKind::UniqueViolation,
                            _,
                        ) => RepoError::IdempotencyConflict,
                        other => RepoError::Query(other),
                    })?;
                let ingestion_id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(conn)?;
                for item in items {
                    diesel::insert_into(schema::ingestion_artifacts::table)
                        .values(NewIngestionArtifact {
                            ingestion_id,
                            client_file_id: item.client_file_id,
                            artifact_id: item.artifact_id,
                            file_name: item.file_name,
                            relative_path: item.relative_path,
                            media_type: item.media_type,
                            role: "uploaded".to_string(),
                            source_artifact_id: None,
                            sheet_name: None,
                            status: "accepted".to_string(),
                            error_code: None,
                            error_message: None,
                            created_at_ms: draft.created_at_ms,
                            updated_at_ms: draft.updated_at_ms,
                        })
                        .execute(conn)?;
                }
                let ingestion = schema::ingestions::table
                    .find(ingestion_id)
                    .first::<Ingestion>(conn)?;
                load_record(conn, ingestion)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, scope: &AccessScope, id: i32) -> RepoResult<IngestionRecord> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let ingestion = schema::ingestions::table
                .inner_join(schema::workspaces::table.inner_join(schema::sessions::table))
                .filter(schema::ingestions::id.eq(id))
                .filter(schema::sessions::user_id.eq(&scope.key))
                .select(schema::ingestions::all_columns)
                .first::<Ingestion>(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            load_record(&mut conn, ingestion)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_active(
        &self,
        scope: &AccessScope,
        workspace_id: i32,
    ) -> RepoResult<Vec<IngestionRecord>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            ensure_workspace(&mut conn, &scope, workspace_id)?;
            let rows = schema::ingestions::table
                .filter(schema::ingestions::workspace_id.eq(workspace_id))
                .filter(schema::ingestions::status.ne_all([
                    "succeeded",
                    "completed_with_errors",
                    "failed",
                ]))
                .order((
                    schema::ingestions::created_at_ms.asc(),
                    schema::ingestions::id.asc(),
                ))
                .load::<Ingestion>(&mut conn)?;
            rows.into_iter()
                .map(|row| load_record(&mut conn, row))
                .collect()
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn set_ingestion_status(
        &self,
        id: i32,
        status: &str,
        stages_json: String,
        published_version_id: Option<i32>,
        error: Option<(String, String)>,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let status = status.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let (error_code, error_message) = error.unzip();
            diesel::update(schema::ingestions::table.find(id))
                .set((
                    schema::ingestions::status.eq(status),
                    schema::ingestions::stages_json.eq(stages_json),
                    schema::ingestions::published_version_id.eq(published_version_id),
                    schema::ingestions::error_code.eq(error_code),
                    schema::ingestions::error_message.eq(error_message),
                    schema::ingestions::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn set_item_status(
        &self,
        item_id: i32,
        status: &str,
        error: Option<(String, String)>,
        now_ms: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let status = status.to_string();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let (error_code, error_message) = error.unzip();
            diesel::update(schema::ingestion_artifacts::table.find(item_id))
                .set((
                    schema::ingestion_artifacts::status.eq(status),
                    schema::ingestion_artifacts::error_code.eq(error_code),
                    schema::ingestion_artifacts::error_message.eq(error_message),
                    schema::ingestion_artifacts::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)?;
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn add_derived(
        &self,
        ingestion_id: i32,
        source: &IngestionArtifact,
        artifact_id: i32,
        file_name: String,
        relative_path: Option<String>,
        role: String,
        sheet_name: Option<String>,
        now_ms: i64,
    ) -> RepoResult<IngestionArtifact> {
        let pool = self.pool.clone();
        let source = source.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            diesel::insert_into(schema::ingestion_artifacts::table)
                .values(NewIngestionArtifact {
                    ingestion_id,
                    client_file_id: source.client_file_id,
                    artifact_id,
                    file_name,
                    relative_path,
                    media_type: (role == "normalized")
                        .then(|| "application/vnd.apache.parquet".to_string()),
                    role,
                    source_artifact_id: Some(source.artifact_id),
                    sheet_name,
                    status: "ready".to_string(),
                    error_code: None,
                    error_message: None,
                    created_at_ms: now_ms,
                    updated_at_ms: now_ms,
                })
                .execute(&mut conn)?;
            let id: i32 =
                diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                    .get_result(&mut conn)?;
            schema::ingestion_artifacts::table
                .find(id)
                .first(&mut conn)
                .map_err(Into::into)
        })
        .await
        .map_err(RepoError::Join)?
    }
}

fn ensure_workspace(
    conn: &mut SqliteConnection,
    scope: &AccessScope,
    workspace_id: i32,
) -> RepoResult<()> {
    let exists = schema::workspaces::table
        .inner_join(schema::sessions::table)
        .filter(schema::workspaces::id.eq(workspace_id))
        .filter(schema::sessions::user_id.eq(&scope.key))
        .select(schema::workspaces::id)
        .first::<i32>(conn)
        .optional()?
        .is_some();
    if exists {
        Ok(())
    } else {
        Err(RepoError::NotFound)
    }
}

fn load_record(conn: &mut SqliteConnection, ingestion: Ingestion) -> RepoResult<IngestionRecord> {
    let session_id = schema::workspaces::table
        .find(ingestion.workspace_id)
        .select(schema::workspaces::session_id)
        .first(conn)?;
    let rows = schema::ingestion_artifacts::table
        .inner_join(
            schema::artifacts::table
                .on(schema::artifacts::id.eq(schema::ingestion_artifacts::artifact_id)),
        )
        .filter(schema::ingestion_artifacts::ingestion_id.eq(ingestion.id))
        .order(schema::ingestion_artifacts::id.asc())
        .select((
            schema::ingestion_artifacts::all_columns,
            schema::artifacts::all_columns,
        ))
        .load::<(IngestionArtifact, Artifact)>(conn)?;
    Ok(IngestionRecord {
        ingestion,
        items: rows,
        session_id,
    })
}
