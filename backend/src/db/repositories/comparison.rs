use diesel::prelude::*;

use super::{RepoError, RepoResult};
use crate::auth::UserId;
use crate::db::models::{NewRunComparison, RunComparison};
use crate::db::{DbPool, schema};

#[derive(Clone)]
pub struct ComparisonRepo {
    pool: DbPool,
}

impl ComparisonRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }

    pub async fn create(&self, user: &UserId, new: NewRunComparison) -> RepoResult<RunComparison> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<_, RepoError, _>(|conn| {
                schema::sessions::table
                    .find(new.session_id)
                    .filter(schema::sessions::user_id.eq(&user))
                    .select(schema::sessions::id)
                    .first::<i32>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                for id in [new.baseline_run_id, new.candidate_run_id] {
                    schema::simulation_runs::table
                        .find(id)
                        .filter(schema::simulation_runs::session_id.eq(new.session_id))
                        .filter(schema::simulation_runs::status.eq("succeeded"))
                        .filter(schema::simulation_runs::deleted_at_ms.is_null())
                        .select(schema::simulation_runs::id)
                        .first::<i32>(conn)
                        .optional()?
                        .ok_or(RepoError::NotFound)?;
                }
                schema::result_analysis_runs::table
                    .find(new.analysis_run_id)
                    .filter(schema::result_analysis_runs::session_id.eq(new.session_id))
                    .filter(schema::result_analysis_runs::status.eq("succeeded"))
                    .select(schema::result_analysis_runs::id)
                    .first::<i32>(conn)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                diesel::insert_into(schema::run_comparisons::table)
                    .values(&new)
                    .execute(conn)?;
                let id: i32 = diesel::select(diesel::dsl::sql::<diesel::sql_types::Integer>(
                    "last_insert_rowid()",
                ))
                .get_result(conn)?;
                Ok(schema::run_comparisons::table.find(id).first(conn)?)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }

    pub async fn find(&self, user: &UserId, id: i32) -> RepoResult<RunComparison> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            schema::run_comparisons::table
                .inner_join(schema::sessions::table)
                .filter(schema::run_comparisons::id.eq(id))
                .filter(schema::sessions::user_id.eq(user))
                .select(schema::run_comparisons::all_columns)
                .first(&mut conn)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }
}
