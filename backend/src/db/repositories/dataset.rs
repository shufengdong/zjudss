use super::{RepoError, RepoResult};
use crate::auth::AccessScope;
use crate::db::models::{DatasetView, ResultDataset};
use crate::db::{DbPool, schema};
use diesel::prelude::*;

#[derive(Clone)]
pub struct DatasetRepo {
    pool: DbPool,
}
impl DatasetRepo {
    pub fn new(pool: DbPool) -> Self {
        Self { pool }
    }
    fn ensure_scope(
        conn: &mut SqliteConnection,
        scope: &AccessScope,
        d: &ResultDataset,
    ) -> RepoResult<()> {
        let sessions = schema::sessions::table
            .filter(schema::sessions::access_scope_kind.eq(&scope.kind))
            .filter(schema::sessions::access_scope_key.eq(&scope.key))
            .select(schema::sessions::id);
        let ok = if let Some(id) = d.task_id {
            schema::simulation_tasks::table
                .filter(schema::simulation_tasks::id.eq(id))
                .filter(schema::simulation_tasks::session_id.eq_any(sessions))
                .select(schema::simulation_tasks::id)
                .first::<i32>(conn)
                .optional()?
                .is_some()
        } else if let Some(id) = d.analysis_run_id {
            schema::result_analysis_runs::table
                .filter(schema::result_analysis_runs::id.eq(id))
                .filter(schema::result_analysis_runs::session_id.eq_any(sessions))
                .select(schema::result_analysis_runs::id)
                .first::<i32>(conn)
                .optional()?
                .is_some()
        } else {
            false
        };
        if ok { Ok(()) } else { Err(RepoError::NotFound) }
    }
    pub async fn find(&self, scope: &AccessScope, id: i32) -> RepoResult<ResultDataset> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            let d = schema::result_datasets::table
                .find(id)
                .first::<ResultDataset>(&mut c)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            Self::ensure_scope(&mut c, &scope, &d)?;
            Ok(d)
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn list_task(
        &self,
        scope: &AccessScope,
        task_id: i32,
    ) -> RepoResult<Vec<ResultDataset>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            let sessions = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            schema::simulation_tasks::table
                .filter(schema::simulation_tasks::id.eq(task_id))
                .filter(schema::simulation_tasks::session_id.eq_any(sessions))
                .select(schema::simulation_tasks::id)
                .first::<i32>(&mut c)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            Ok(schema::result_datasets::table
                .filter(schema::result_datasets::task_id.eq(task_id))
                .order(schema::result_datasets::id.asc())
                .load(&mut c)?)
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn views(
        &self,
        scope: &AccessScope,
        dataset_id: i32,
    ) -> RepoResult<Vec<DatasetView>> {
        self.find(scope, dataset_id).await?;
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            Ok(schema::dataset_views::table
                .filter(schema::dataset_views::dataset_id.eq(dataset_id))
                .order(schema::dataset_views::id.asc())
                .load(&mut c)?)
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn list_analysis(
        &self,
        scope: &AccessScope,
        analysis_id: i32,
    ) -> RepoResult<Vec<ResultDataset>> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            let sessions = schema::sessions::table
                .filter(schema::sessions::access_scope_kind.eq(scope.kind))
                .filter(schema::sessions::access_scope_key.eq(scope.key))
                .select(schema::sessions::id);
            schema::result_analysis_runs::table
                .filter(schema::result_analysis_runs::id.eq(analysis_id))
                .filter(schema::result_analysis_runs::session_id.eq_any(sessions))
                .select(schema::result_analysis_runs::id)
                .first::<i32>(&mut c)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            Ok(schema::result_datasets::table
                .filter(schema::result_datasets::analysis_run_id.eq(analysis_id))
                .order(schema::result_datasets::id.asc())
                .load(&mut c)?)
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn find_view(
        &self,
        scope: &AccessScope,
        id: i32,
    ) -> RepoResult<(DatasetView, ResultDataset)> {
        let pool = self.pool.clone();
        let scope = scope.clone();
        tokio::task::spawn_blocking(move || {
            let mut c = pool.get().map_err(RepoError::Pool)?;
            let view = schema::dataset_views::table
                .find(id)
                .first::<DatasetView>(&mut c)
                .optional()?
                .ok_or(RepoError::NotFound)?;
            let d = schema::result_datasets::table
                .find(view.dataset_id)
                .first::<ResultDataset>(&mut c)?;
            Self::ensure_scope(&mut c, &scope, &d)?;
            Ok((view, d))
        })
        .await
        .map_err(RepoError::Join)?
    }
    pub async fn session_id(&self, scope: &AccessScope, dataset_id: i32) -> RepoResult<i32> {
        let dataset = self.find(scope, dataset_id).await?;
        let pool = self.pool.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            if let Some(id) = dataset.task_id {
                Ok(schema::simulation_tasks::table
                    .find(id)
                    .select(schema::simulation_tasks::session_id)
                    .first(&mut conn)?)
            } else if let Some(id) = dataset.analysis_run_id {
                Ok(schema::result_analysis_runs::table
                    .find(id)
                    .select(schema::result_analysis_runs::session_id)
                    .first(&mut conn)?)
            } else {
                Err(RepoError::NotFound)
            }
        })
        .await
        .map_err(RepoError::Join)?
    }
}
