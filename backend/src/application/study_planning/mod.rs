mod contracts;
mod facts;
mod policy;

use std::sync::Arc;

use crate::application::{ArtifactService, now_ms};
use crate::auth::CurrentUser;
use crate::db::repositories::{RepoError, VersionRepo, VersionRepoError};
use crate::engine_resources::{EngineResourceSnapshot, FacadeContract, FacadeSearchResult};

pub use contracts::*;
pub use policy::*;

#[derive(Clone)]
pub struct StudyPlanningService {
    versions: VersionRepo,
    artifacts: ArtifactService,
    engine_snapshot: Arc<EngineResourceSnapshot>,
}

impl StudyPlanningService {
    pub fn new(
        versions: VersionRepo,
        artifacts: ArtifactService,
        engine_snapshot: Arc<EngineResourceSnapshot>,
    ) -> Self {
        Self {
            versions,
            artifacts,
            engine_snapshot,
        }
    }

    pub fn search(&self, query: &str, explicit_function: Option<&str>) -> FacadeSearchResult {
        self.engine_snapshot
            .facades
            .search(query, explicit_function)
    }

    pub fn get(&self, facade_id: &str) -> Result<FacadeContract, StudyPlanningError> {
        if let Some(facade) = self.engine_snapshot.facades.get_enabled(facade_id) {
            return Ok(facade.clone());
        }
        if let Some(diagnostic) = self.engine_snapshot.facades.diagnostic(facade_id) {
            return Err(StudyPlanningError::InvalidCatalog(format!(
                "{}: {}",
                diagnostic.code, diagnostic.message
            )));
        }
        Err(StudyPlanningError::NotFound)
    }

    pub async fn plan(
        &self,
        user: &CurrentUser,
        session_id: i32,
        request: StudyRequest,
    ) -> Result<StudyPlan, StudyPlanningError> {
        let workspace = self
            .versions
            .workspace(&user.id, session_id, now_ms())
            .await
            .map_err(map_version_error)?;
        let facts = match workspace.active.as_ref() {
            Some(active) => {
                facts::extract_active_model_facts(
                    user,
                    active,
                    &self.artifacts,
                    &self.engine_snapshot.id,
                )
                .await?
            }
            None => facts::empty_model_facts(&self.engine_snapshot.id),
        };
        Ok(evaluate(&request, &facts, &self.engine_snapshot.facades))
    }
}

fn map_version_error(error: VersionRepoError) -> StudyPlanningError {
    match error {
        VersionRepoError::Repository(RepoError::NotFound) => StudyPlanningError::NotFound,
        VersionRepoError::Repository(error) => {
            StudyPlanningError::ModelReadFailed(error.to_string())
        }
        _ => StudyPlanningError::ModelReadFailed("工作区状态无效".into()),
    }
}
