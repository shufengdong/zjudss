use super::{RepoError, RepoResult};
use crate::{
    auth::AccessScope,
    db::models::{NewTaskComparison, TaskComparison},
    db::{DbPool, schema},
};
use diesel::prelude::*;
#[derive(Clone)]
pub struct ComparisonRepo {
    pool: DbPool,
}
impl ComparisonRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
    pub async fn create(
        &self,
        scope: &AccessScope,
        new: NewTaskComparison,
    ) -> RepoResult<TaskComparison> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            c.immediate_transaction::<_, RepoError, _>(|c| {
                schema::sessions::table
                    .filter(schema::sessions::id.eq(new.session_id))
                    .filter(schema::sessions::user_id.eq(&scope.key))
                    .select(schema::sessions::id)
                    .first::<i32>(c)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                for id in [new.baseline_task_id, new.candidate_task_id] {
                    schema::simulation_tasks::table
                        .filter(schema::simulation_tasks::id.eq(id))
                        .filter(schema::simulation_tasks::session_id.eq(new.session_id))
                        .filter(schema::simulation_tasks::status.eq("succeeded"))
                        .select(schema::simulation_tasks::id)
                        .first::<i32>(c)
                        .optional()?
                        .ok_or(RepoError::NotFound)?;
                }
                schema::result_analysis_runs::table
                    .filter(schema::result_analysis_runs::id.eq(new.analysis_run_id))
                    .filter(schema::result_analysis_runs::session_id.eq(new.session_id))
                    .filter(schema::result_analysis_runs::status.eq("succeeded"))
                    .select(schema::result_analysis_runs::id)
                    .first::<i32>(c)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                diesel::insert_into(schema::task_comparisons::table)
                    .values(new)
                    .execute(c)?;
                let id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(c)?;
                Ok(schema::task_comparisons::table.find(id).first(c)?)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn find(&self, scope: &AccessScope, id: i32) -> RepoResult<TaskComparison> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            schema::task_comparisons::table
                .inner_join(schema::sessions::table)
                .filter(schema::task_comparisons::id.eq(id))
                .filter(schema::sessions::user_id.eq(scope.key))
                .select(schema::task_comparisons::all_columns)
                .first(&mut c)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }
}
