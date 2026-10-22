use super::{RepoError, RepoResult};
use crate::auth::AccessScope;
use crate::db::models::{
    DatasetView, NewDatasetView, NewResultAnalysisRun, NewResultDataset, ResultAnalysisInput,
    ResultAnalysisRun, ResultDataset,
};
use crate::db::{DbPool, schema};
use diesel::prelude::*;

#[derive(Clone)]
pub struct AnalysisRepo {
    pool: DbPool,
}
impl AnalysisRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
    pub async fn create(
        &self,
        scope: &AccessScope,
        run: NewResultAnalysisRun,
        inputs: Vec<ResultAnalysisInput>,
    ) -> RepoResult<ResultAnalysisRun> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            c.transaction::<_, RepoError, _>(|c| {
                let session_id = run.session_id;
                schema::sessions::table
                    .filter(schema::sessions::id.eq(session_id))
                    .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
                    .filter(schema::sessions::access_scope_key.eq(&scope.key))
                    .select(schema::sessions::id)
                    .first::<i32>(c)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                diesel::insert_into(schema::result_analysis_runs::table)
                    .values(run)
                    .execute(c)?;
                let id: i32 =
                    diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                        .get_result(c)?;
                for mut input in inputs {
                    input.analysis_run_id = id;
                    let d = schema::result_datasets::table
                        .find(input.dataset_id)
                        .first::<ResultDataset>(c)
                        .optional()?
                        .ok_or(RepoError::NotFound)?;
                    let owned = if let Some(task) = d.task_id {
                        schema::simulation_tasks::table
                            .filter(schema::simulation_tasks::id.eq(task))
                            .filter(schema::simulation_tasks::session_id.eq(session_id))
                            .select(schema::simulation_tasks::id)
                            .first::<i32>(c)
                            .optional()?
                            .is_some()
                    } else if let Some(parent) = d.analysis_run_id {
                        schema::result_analysis_runs::table
                            .filter(schema::result_analysis_runs::id.eq(parent))
                            .filter(schema::result_analysis_runs::session_id.eq(session_id))
                            .select(schema::result_analysis_runs::id)
                            .first::<i32>(c)
                            .optional()?
                            .is_some()
                    } else {
                        false
                    };
                    if !owned {
                        return Err(RepoError::NotFound);
                    }
                    diesel::insert_into(schema::result_analysis_inputs::table)
                        .values(input)
                        .execute(c)?;
                }
                Ok(schema::result_analysis_runs::table.find(id).first(c)?)
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn set_stage(
        &self,
        scope: &AccessScope,
        id: i32,
        stage: &str,
        now: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        let stage = stage.to_string();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            let sessions = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            let n = diesel::update(
                schema::result_analysis_runs::table
                    .filter(schema::result_analysis_runs::id.eq(id))
                    .filter(schema::result_analysis_runs::session_id.eq_any(sessions)),
            )
            .set((
                schema::result_analysis_runs::status.eq("running"),
                schema::result_analysis_runs::stage.eq(stage),
                schema::result_analysis_runs::started_at_ms.eq(Some(now)),
            ))
            .execute(&mut c)?;
            if n == 0 {
                Err(RepoError::NotFound)
            } else {
                Ok(())
            }
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn complete(
        &self,
        scope: &AccessScope,
        id: i32,
        datasets: Vec<(NewResultDataset, Vec<NewDatasetView>)>,
        now: i64,
    ) -> RepoResult<(ResultAnalysisRun, Vec<ResultDataset>)> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            c.transaction::<_, RepoError, _>(|c| {
                let sessions = schema::sessions::table
                    .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
                    .filter(schema::sessions::access_scope_key.eq(&scope.key))
                    .select(schema::sessions::id);
                schema::result_analysis_runs::table
                    .filter(schema::result_analysis_runs::id.eq(id))
                    .filter(schema::result_analysis_runs::session_id.eq_any(sessions))
                    .select(schema::result_analysis_runs::id)
                    .first::<i32>(c)
                    .optional()?
                    .ok_or(RepoError::NotFound)?;
                for (dataset, views) in datasets {
                    diesel::insert_into(schema::result_datasets::table)
                        .values(dataset)
                        .execute(c)?;
                    let dataset_id: i32 = diesel::dsl::sql::<diesel::sql_types::Integer>(
                        "SELECT last_insert_rowid()",
                    )
                    .get_result(c)?;
                    for mut view in views {
                        view.dataset_id = dataset_id;
                        diesel::insert_into(schema::dataset_views::table)
                            .values(view)
                            .execute(c)?;
                    }
                }
                diesel::update(schema::result_analysis_runs::table.find(id))
                    .set((
                        schema::result_analysis_runs::status.eq("succeeded"),
                        schema::result_analysis_runs::stage.eq("present"),
                        schema::result_analysis_runs::error_json.eq::<Option<String>>(None),
                        schema::result_analysis_runs::finished_at_ms.eq(now),
                    ))
                    .execute(c)?;
                Ok((
                    schema::result_analysis_runs::table.find(id).first(c)?,
                    schema::result_datasets::table
                        .filter(schema::result_datasets::analysis_run_id.eq(id))
                        .order(schema::result_datasets::id.asc())
                        .load(c)?,
                ))
            })
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn fail(
        &self,
        scope: &AccessScope,
        id: i32,
        stage: &str,
        error: String,
        now: i64,
    ) -> RepoResult<()> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        let stage = stage.to_string();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            let sessions = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            let n = diesel::update(
                schema::result_analysis_runs::table
                    .filter(schema::result_analysis_runs::id.eq(id))
                    .filter(schema::result_analysis_runs::session_id.eq_any(sessions)),
            )
            .set((
                schema::result_analysis_runs::status.eq("failed"),
                schema::result_analysis_runs::stage.eq(stage),
                schema::result_analysis_runs::error_json.eq(error),
                schema::result_analysis_runs::finished_at_ms.eq(now),
            ))
            .execute(&mut c)?;
            if n == 0 {
                Err(RepoError::NotFound)
            } else {
                Ok(())
            }
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn find(&self, scope: &AccessScope, id: i32) -> RepoResult<ResultAnalysisRun> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            let sessions = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            schema::result_analysis_runs::table
                .filter(schema::result_analysis_runs::id.eq(id))
                .filter(schema::result_analysis_runs::session_id.eq_any(sessions))
                .first(&mut c)
                .optional()?
                .ok_or(RepoError::NotFound)
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn create_view(
        &self,
        scope: &AccessScope,
        dataset_id: i32,
        view: NewDatasetView,
    ) -> RepoResult<DatasetView> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            let d = schema::result_datasets::table
                .find(dataset_id)
                .first::<ResultDataset>(&mut c)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            let sessions = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
                .filter(schema::sessions::access_scope_key.eq(&scope.key))
                .select(schema::sessions::id);
            let ok = if let Some(t) = d.task_id {
                schema::simulation_tasks::table
                    .filter(schema::simulation_tasks::id.eq(t))
                    .filter(schema::simulation_tasks::session_id.eq_any(sessions))
                    .select(schema::simulation_tasks::id)
                    .first::<i32>(&mut c)
                    .optional()?
                    .is_some()
            } else if let Some(a) = d.analysis_run_id {
                schema::result_analysis_runs::table
                    .filter(schema::result_analysis_runs::id.eq(a))
                    .filter(schema::result_analysis_runs::session_id.eq_any(sessions))
                    .select(schema::result_analysis_runs::id)
                    .first::<i32>(&mut c)
                    .optional()?
                    .is_some()
            } else {
                false
            };
            if !ok {
                return Err(RepoError::NotFound);
            }
            diesel::insert_into(schema::dataset_views::table)
                .values(view)
                .execute(&mut c)?;
            let id: i32 =
                diesel::dsl::sql::<diesel::sql_types::Integer>("SELECT last_insert_rowid()")
                    .get_result(&mut c)?;
            Ok(schema::dataset_views::table.find(id).first(&mut c)?)
        })
        .await
        .map_err(RepoError::Join)?
    }
}
