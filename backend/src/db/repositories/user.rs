use diesel::prelude::*;

use crate::db::models::User;
use crate::db::{DbPool, schema};

use super::{RepoError, RepoResult};

#[derive(Clone)]
pub struct UserRepo {
    pool: DbPool,
}

impl UserRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn find(&self, user_id: &str) -> RepoResult<User> {
        let pool = self.pool.clone();
        let user_id = user_id.to_owned();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::users::table
                .find(user_id)
                .first(&mut conn)
                .optional()
                .map_err(RepoError::Query)?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }
}
