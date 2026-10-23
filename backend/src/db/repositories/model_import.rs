use diesel::prelude::*;

use crate::auth::UserId;
use crate::db::models::{ModelImport, NewModelImport};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct ModelImportRepo {
    pool: DbPool,
}

impl ModelImportRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, record: NewModelImport) -> RepoResult<ModelImport> {
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction(|conn| {
                diesel::insert_into(schema::model_imports::table)
                    .values(&record)
                    .execute(conn)?;
                schema::model_imports::table
                    .order(schema::model_imports::id.desc())
                    .first(conn)
                    .map_err(RepoError::from)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, user_id: &UserId, id: i32) -> RepoResult<ModelImport> {
        let pool = self.pool.clone();
        let user_id = user_id.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::model_imports::table
                .inner_join(
                    schema::sessions::table
                        .on(schema::sessions::id.eq(schema::model_imports::session_id)),
                )
                .filter(schema::model_imports::id.eq(id))
                .filter(schema::sessions::user_id.eq(user_id))
                .select(schema::model_imports::all_columns)
                .first(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }
}
