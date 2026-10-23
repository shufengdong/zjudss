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

    pub async fn create(&self, record: NewScriptResource) -> RepoResult<ScriptResource> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction(|conn| {
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
}
