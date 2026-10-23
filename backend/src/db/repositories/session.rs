use diesel::prelude::*;

use crate::auth::UserId;
use crate::db::models::{NewSession, Session};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct SessionRepo {
    pool: DbPool,
}

impl SessionRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, user_id: &UserId, title: &str, now_ms: i64) -> RepoResult<Session> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        let title = title.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<Session> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<Session, diesel::result::Error, _>(|conn| {
                diesel::insert_into(schema::sessions::table)
                    .values(NewSession {
                        title,
                        created_at_ms: now_ms,
                        updated_at_ms: now_ms,
                        user_id: user_id.0.clone(),
                    })
                    .execute(conn)?;
                let id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(conn)?;
                schema::sessions::table.find(id).first(conn)
            })
            .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list(&self, user_id: &UserId) -> RepoResult<Vec<Session>> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<Session>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .filter(schema::sessions::user_id.eq(user_id.0))
                .order(schema::sessions::updated_at_ms.desc())
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn touch(&self, user_id: &UserId, session_id: i32, now_ms: i64) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let updated = diesel::update(
                schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::user_id.eq(user_id.0)),
            )
            .set(schema::sessions::updated_at_ms.eq(now_ms))
            .execute(&mut conn)
            .map_err(RepoError::Query)?;
            if updated == 0 {
                return Err(RepoError::NotFound);
            }
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, user_id: &UserId, session_id: i32) -> RepoResult<Option<Session>> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Option<Session>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(user_id.0))
                .first(&mut conn)
                .optional()
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn rename(
        &self,
        user_id: &UserId,
        session_id: i32,
        title: &str,
        now_ms: i64,
    ) -> RepoResult<Session> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        let title = title.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<Session> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let target = schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(&user_id.0));
            let updated = diesel::update(target)
                .set((
                    schema::sessions::title.eq(title),
                    schema::sessions::title_source.eq("user"),
                    schema::sessions::updated_at_ms.eq(now_ms),
                ))
                .execute(&mut conn)
                .map_err(RepoError::Query)?;
            if updated == 0 {
                return Err(RepoError::NotFound);
            }
            schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::user_id.eq(user_id.0))
                .first(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn delete(&self, user_id: &UserId, session_id: i32) -> RepoResult<()> {
        let pool = self.pool.clone();
        let user_id = user_id.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let deleted = diesel::delete(
                schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::user_id.eq(user_id.0)),
            )
            .execute(&mut conn)
            .map_err(RepoError::Query)?;
            if deleted == 0 {
                return Err(RepoError::NotFound);
            }
            Ok(())
        })
        .await
        .map_err(RepoError::Join)?
    }
}
