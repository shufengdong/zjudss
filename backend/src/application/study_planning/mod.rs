mod contracts;
mod facts;
mod policy;

use std::sync::Arc;

use crate::application::{ArtifactService, now_ms};
use crate::auth::CurrentUser;
use crate::db::repositories::{RepoError, VersionRepo, VersionRepoError};
use crate::engine_resources::EngineResourceSnapshot;

pub use contracts::*;
pub use policy::*;

#[derive(Clone)]
pub struct StudyPlanningService {
    versions: VersionRepo,
    artifacts: ArtifactService,
    engine_snapshot: Arc<EngineResourceSnapshot>,
    functions: Result<StudyFunctionCatalog, StudyPlanningError>,
}

impl StudyPlanningService {
    pub fn new(
        versions: VersionRepo,
        artifacts: ArtifactService,
        engine_snapshot: Arc<EngineResourceSnapshot>,
    ) -> Self {
        let functions = StudyFunctionCatalog::from_value(&engine_snapshot.functions_json);
        Self {
            versions,
            artifacts,
            engine_snapshot,
            functions,
        }
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
        let functions = self.functions.as_ref().map_err(Clone::clone)?;
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
        Ok(evaluate(&request, &facts, functions))
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
