use serde_json::{Value, json};

use crate::application::dataframe_reader::{DataFrameColumn, DataFrameReadSource};
use crate::application::{ArtifactError, ArtifactRef, ArtifactService};
use crate::auth::CurrentUser;
use crate::db::repositories::{RepoError, ResultRepo, RunDataset};

#[derive(Debug)]
pub(crate) enum ResultSourceError {
    NotFound,
    Repository(RepoError),
    Artifact(ArtifactError),
    Stored(String),
}

impl std::fmt::Display for ResultSourceError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "资源不存在"),
            Self::Repository(error) => write!(formatter, "{error}"),
            Self::Artifact(error) => write!(formatter, "{error}"),
            Self::Stored(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for ResultSourceError {}

#[derive(Debug, Clone)]
pub(crate) struct ResultSource {
    pub dataset: RunDataset,
    pub session_id: i32,
    pub artifact: ArtifactRef,
    pub columns: Vec<DataFrameColumn>,
    pub read_source: DataFrameReadSource,
}

impl ResultSource {
    fn from_parts(
        dataset: RunDataset,
        session_id: i32,
        artifact: ArtifactRef,
    ) -> Result<Self, ResultSourceError> {
        let artifact_id = dataset.artifact_id.ok_or(ResultSourceError::NotFound)?;
        if artifact_id != artifact.id {
            return Err(ResultSourceError::NotFound);
        }
        let columns = serde_json::from_str(&dataset.schema_json)
            .map_err(|error| ResultSourceError::Stored(error.to_string()))?;
        let read_source = DataFrameReadSource {
            resource_key: format!("dataset:{}", dataset.id),
            artifact_id,
            schema_json: dataset.schema_json.clone(),
            row_count: dataset.row_count,
            content_hash: artifact.sha256.clone(),
        };
        Ok(Self {
            dataset,
            session_id,
            artifact,
            columns,
            read_source,
        })
    }

    pub(crate) fn producer(&self) -> Value {
        if let Some(run_id) = self.dataset.run_id {
            json!({"kind":"run","runId":run_id.to_string()})
        } else {
            json!({
                "kind":"analysis",
                "analysisRunId":self.dataset.analysis_run_id.map(|id| id.to_string())
            })
        }
    }
}

#[derive(Clone)]
pub(crate) struct ResultSourceService {
    results: ResultRepo,
    artifacts: ArtifactService,
}

impl ResultSourceService {
    pub(crate) fn new(results: ResultRepo, artifacts: ArtifactService) -> Self {
        Self { results, artifacts }
    }

    pub(crate) async fn resolve(
        &self,
        principal: &CurrentUser,
        dataset_id: i32,
    ) -> Result<ResultSource, ResultSourceError> {
        let dataset = self
            .results
            .find_live(&principal.id, dataset_id)
            .await
            .map_err(repository_error)?;
        let session_id = self
            .results
            .session_id(&principal.id, dataset_id)
            .await
            .map_err(repository_error)?;
        let artifact_id = dataset.artifact_id.ok_or(ResultSourceError::NotFound)?;
        let artifact = self
            .artifacts
            .find_owned(principal.storage_scope_id, artifact_id)
            .await
            .map_err(artifact_error)?
            .into();
        ResultSource::from_parts(dataset, session_id, artifact)
    }
}

fn repository_error(error: RepoError) -> ResultSourceError {
    match error {
        RepoError::NotFound => ResultSourceError::NotFound,
        other => ResultSourceError::Repository(other),
    }
}

fn artifact_error(error: ArtifactError) -> ResultSourceError {
    ResultSourceError::Artifact(error)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    fn dataset(run_id: Option<i32>, analysis_run_id: Option<i32>) -> RunDataset {
        RunDataset {
            id: 7,
            run_id,
            analysis_run_id,
            artifact_id: Some(11),
            result_name: "power".into(),
            schema_json: json!([{
                "name":"load","dtype":"Float64","nullable":false,"unit":"MW"
            }])
            .to_string(),
            schema_hash: "schema-hash".into(),
            row_count: 3,
            size_bytes: 20,
            created_at_ms: 1,
            deleted_at_ms: None,
            tombstone_json: None,
        }
    }

    fn artifact() -> ArtifactRef {
        ArtifactRef {
            id: 11,
            sha256: "content-hash".into(),
            size_bytes: 20,
            relative_path: "objects/content-hash".into(),
        }
    }

    #[test]
    fn source_uses_artifact_content_identity_and_run_producer() {
        let source = ResultSource::from_parts(dataset(Some(5), None), 3, artifact()).unwrap();

        assert_eq!(source.session_id, 3);
        assert_eq!(source.artifact.sha256, "content-hash");
        assert_eq!(source.columns[0].name, "load");
        assert_eq!(source.read_source.resource_key, "dataset:7");
        assert_eq!(source.read_source.artifact_id, 11);
        assert_eq!(source.read_source.content_hash, "content-hash");
        assert_eq!(source.producer(), json!({"kind":"run","runId":"5"}));
    }

    #[test]
    fn source_describes_analysis_producer() {
        let source = ResultSource::from_parts(dataset(None, Some(9)), 3, artifact()).unwrap();

        assert_eq!(
            source.producer(),
            json!({"kind":"analysis","analysisRunId":"9"})
        );
    }

    #[test]
    fn source_rejects_missing_artifact_and_malformed_stored_schema() {
        let mut missing = dataset(Some(5), None);
        missing.artifact_id = None;
        assert!(matches!(
            ResultSource::from_parts(missing, 3, artifact()),
            Err(ResultSourceError::NotFound)
        ));

        let mut malformed = dataset(Some(5), None);
        malformed.schema_json = "not-json".into();
        assert!(matches!(
            ResultSource::from_parts(malformed, 3, artifact()),
            Err(ResultSourceError::Stored(_))
        ));
    }

    #[test]
    fn artifact_not_found_remains_distinguishable_from_dataset_not_found() {
        assert!(matches!(
            artifact_error(ArtifactError::NotFound),
            ResultSourceError::Artifact(ArtifactError::NotFound)
        ));
    }
}
