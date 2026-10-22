use diesel::prelude::*;

use crate::auth::AccessScope;
use crate::db::models::{Message, NewMessage};
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct MessageRepo {
    pool: DbPool,
}

impl MessageRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, scope: &AccessScope, new: NewMessage) -> RepoResult<Message> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Message> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction::<Message, diesel::result::Error, _>(|conn| {
                schema::sessions::table
                    .filter(schema::sessions::id.eq(new.session_id))
                    .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
                    .filter(schema::sessions::access_scope_key.eq(&scope.key))
                    .select(schema::sessions::id)
                    .first::<i32>(conn)?;
                diesel::insert_into(schema::messages::table)
                    .values(new)
                    .execute(conn)?;
                let id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(conn)?;
                schema::messages::table.find(id).first(conn)
            })
            .map_err(|error| match error {
                diesel::result::Error::NotFound => RepoError::NotFound,
                other => RepoError::Query(other),
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn list_by_session(
        &self,
        scope: &AccessScope,
        session_id: i32,
    ) -> RepoResult<Vec<Message>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || -> RepoResult<Vec<Message>> {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            let exists = schema::sessions::table
                .filter(schema::sessions::id.eq(session_id))
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id)
                .first::<i32>(&mut conn)
                .optional()
                .map_err(RepoError::Query)?;
            if exists.is_none() {
                return Err(RepoError::NotFound);
            }
            schema::messages::table
                .filter(schema::messages::session_id.eq(session_id))
                .order(schema::messages::created_at_ms.asc())
                .load(&mut conn)
                .map_err(RepoError::Query)
        })
        .await
        .map_err(RepoError::Join)?
    }
}
