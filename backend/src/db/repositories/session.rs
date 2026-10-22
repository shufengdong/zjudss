use diesel::prelude::*;

use crate::auth::AccessScope;
use crate::db::models::{NewSession, NewStorageScope, Session};
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

    pub async fn create(
        &self,
        scope: &AccessScope,
        title: &str,
        now_ms: i64,
    ) -> RepoResult<Session> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        let title = title.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<Session> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction::<Session, diesel::result::Error, _>(|conn| {
                diesel::insert_into(schema::storage_scopes::table)
                    .values(NewStorageScope {
                        kind: scope.kind.clone(),
                        scope_key: scope.key.clone(),
                        created_at_ms: now_ms,
                    })
                    .on_conflict((
                        schema::storage_scopes::kind,
                        schema::storage_scopes::scope_key,
                    ))
                    .do_nothing()
                    .execute(conn)?;
                let storage_scope_id: i32 = schema::storage_scopes::table
                    .filter(schema::storage_scopes::kind.eq(&scope.kind))
                    .filter(schema::storage_scopes::scope_key.eq(&scope.key))
                    .select(schema::storage_scopes::id)
                    .first(conn)?;
                diesel::insert_into(schema::sessions::table)
                    .values(NewSession {
                        title,
                        created_at_ms: now_ms,
                        updated_at_ms: now_ms,
                        access_scope_kind: scope.kind.clone(),
                        access_scope_key: scope.key.clone(),
                        storage_scope_id: Some(storage_scope_id),
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

    pub async fn list(&self, scope: &AccessScope) -> RepoResult<Vec<Session>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<Session>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .order(schema::sessions::updated_at_ms.desc())
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn touch(&self, scope: &AccessScope, session_id: i32, now_ms: i64) -> RepoResult<()> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let updated = diesel::update(
                schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                    .filter(schema::sessions::access_scope_key.eq(scope.key)),
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

    pub async fn find(&self, scope: &AccessScope, session_id: i32) -> RepoResult<Option<Session>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Option<Session>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .first(&mut conn)
                .optional()
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn rename(
        &self,
        scope: &AccessScope,
        session_id: i32,
        title: &str,
        now_ms: i64,
    ) -> RepoResult<Session> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        let title = title.to_string();
        tokio::task::spawn_blocking(move || -> RepoResult<Session> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let target = schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
                .filter(schema::sessions::access_scope_key.eq(&scope.key));
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
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .first(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn delete(&self, scope: &AccessScope, session_id: i32) -> RepoResult<()> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<()> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let deleted = diesel::delete(
                schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                    .filter(schema::sessions::access_scope_key.eq(scope.key)),
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
