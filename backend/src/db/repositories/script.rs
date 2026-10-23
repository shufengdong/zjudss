use diesel::prelude::*;

use crate::auth::UserId;
use crate::db::models::{NewScriptResource, ScriptResource};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct ScriptRepo {
    pool: DbPool,
}

impl ScriptRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(
        &self,
        user_id: &UserId,
        record: NewScriptResource,
    ) -> RepoResult<ScriptResource> {
        let pool = self.pool.clone();
        let user_id = user_id.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction(|conn| {
                schema::sessions::table
                    .filter(schema::sessions::id.eq(record.session_id))
                    .filter(schema::sessions::user_id.eq(user_id))
                    .select(schema::sessions::id)
                    .first::<i32>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                diesel::insert_into(schema::script_resources::table)
                    .values(&record)
                    .execute(conn)?;
                schema::script_resources::table
                    .order(schema::script_resources::id.desc())
                    .first(conn)
                    .map_err(RepoError::from)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, user_id: &UserId, id: i32) -> RepoResult<ScriptResource> {
        let pool = self.pool.clone();
        let user_id = user_id.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::script_resources::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::script_resources::session_id)),
                )
                .filter(schema::script_resources::id.eq(id))
                .filter(schema::sessions::user_id.eq(user_id))
                .select(schema::script_resources::all_columns)
                .first(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list(&self, user_id: &UserId, session_id: i32) -> RepoResult<Vec<ScriptResource>> {
        let pool = self.pool.clone();
        let user_id = user_id.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let owns = schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(user_id))
                .select(schema::sessions::id)
                .first::<i32>(&mut conn)
                .optional()?
                .is_some();
            if !owns {
                return Err(RepoError::NotFound);
            }
            schema::script_resources::table
                .filter(schema::script_resources::session_id.eq(session_id))
                .order(schema::script_resources::id.asc())
                .load(&mut conn)
                .map_err(RepoError::from)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn active_aliases(
        &self,
        user_id: &UserId,
        session_id: i32,
    ) -> RepoResult<Option<Vec<String>>> {
        let pool = self.pool.clone();
        let user_id = user_id.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            #[derive(diesel::QueryableByName)]
            struct Alias { #[diesel(sql_type=diesel::sql_types::Text)] table_name: String }
            #[derive(diesel::QueryableByName)]
            struct Active { #[diesel(sql_type=diesel::sql_types::Nullable<diesel::sql_types::Integer>)] active_version_id: Option<i32> }
            let active = diesel::sql_query("SELECT w.active_version_id FROM workspaces w JOIN sessions s ON s.id=w.session_id WHERE w.session_id=?1 AND s.user_id=?2")
                .bind::<diesel::sql_types::Integer,_>(session_id)
                .bind::<diesel::sql_types::Text,_>(user_id)
                .get_result::<Active>(&mut conn)
                .optional()?;
            let Some(active) = active else {
                let owns = schema::sessions::table.find(session_id).select(schema::sessions::id).first::<i32>(&mut conn).optional()?;
                return owns.map(|_| None).ok_or(RepoError::NotFound);
            };
            let Some(version_id) = active.active_version_id else { return Ok(None); };
            let aliases = diesel::sql_query("SELECT table_name FROM model_dataframes WHERE version_id=?1 ORDER BY table_name")
                .bind::<diesel::sql_types::Integer,_>(version_id)
                .load::<Alias>(&mut conn)?
                .into_iter().map(|row| row.table_name).collect();
            Ok(Some(aliases))
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn artifact_ids(&self, user_id: &UserId, session_id: i32) -> RepoResult<Vec<i32>> {
        Ok(self
            .list(user_id, session_id)
            .await?
            .into_iter()
            .map(|script| script.artifact_id)
            .collect())
    }
}
