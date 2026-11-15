use std::path::{Path, PathBuf};

use serde_json::json;
use sha2::{Digest, Sha256};

use super::{AnalysisError, AnalysisInput, AnalysisInputSource};
use crate::adapters::NamedInput;
use crate::application::ArtifactService;
use crate::application::result_sources::{ResultSource, ResultSourceService};
use crate::auth::CurrentUser;
use crate::db::models::ResultAnalysisInput;
use crate::db::repositories::{DataResourceRepo, ResultRepo, VersionRepo, VersionRepoError};

#[derive(Debug, Clone)]
pub(super) struct ResolvedAnalysisInput {
    snapshot: ResultAnalysisInput,
    artifact_id: i32,
    input_key: String,
}

impl ResolvedAnalysisInput {
    fn from_dataset(
        alias: String,
        ordinal: i32,
        source: ResultSource,
    ) -> Result<Self, AnalysisError> {
        let artifact_id = source.artifact.id;
        let dataset_id = source.dataset.id;
        Ok(Self {
            snapshot: ResultAnalysisInput {
                analysis_run_id: 0,
                source_kind: "dataset".into(),
                dataset_id: Some(dataset_id),
                resource_id: None,
                model_version_id: None,
                model_dataframe_id: None,
                alias,
                ordinal,
                source_name: source.dataset.result_name.clone(),
                source_schema_json: source.dataset.schema_json.clone(),
                source_schema_hash: source.dataset.schema_hash.clone(),
                source_content_hash: source.artifact.sha256.clone(),
                source_producer_json: source.producer().to_string(),
            },
            artifact_id,
            input_key: format!("dataset-{dataset_id}"),
        })
    }

    fn from_active_model(
        alias: String,
        ordinal: i32,
        source: crate::db::repositories::VersionDataFrameReadRow,
    ) -> Self {
        let schema_hash = hex::encode(Sha256::digest(source.schema_json.as_bytes()));
        let producer = json!({
            "kind": "active_model",
            "modelVersionId": source.version_id,
            "modelContentHash": source.model_content_hash,
            "modelDataframeId": source.id,
            "tableName": source.table_name,
            "contentHash": source.content_hash,
        });
        Self {
            artifact_id: source.artifact_id,
            input_key: format!("model-{}", source.id),
            snapshot: ResultAnalysisInput {
                analysis_run_id: 0,
                source_kind: "active_model".into(),
                dataset_id: None,
                resource_id: None,
                model_version_id: Some(source.version_id),
                model_dataframe_id: Some(source.id),
                alias,
                ordinal,
                source_name: source.table_name,
                source_schema_json: source.schema_json,
                source_schema_hash: schema_hash,
                source_content_hash: source.content_hash,
                source_producer_json: producer.to_string(),
            },
        }
    }

    #[cfg(test)]
    fn artifact_id(&self) -> i32 {
        self.artifact_id
    }

    #[cfg(test)]
    fn input_key(&self) -> &str {
        &self.input_key
    }
}

#[derive(Debug, Clone)]
pub(super) struct ResolvedAnalysisInputs {
    entries: Vec<ResolvedAnalysisInput>,
}

impl ResolvedAnalysisInputs {
    #[cfg(test)]
    fn from_entries(entries: Vec<ResolvedAnalysisInput>) -> Self {
        Self { entries }
    }

    pub(super) async fn resolve(
        inputs: &[AnalysisInput],
        principal: &CurrentUser,
        session_id: i32,
        sources: &ResultSourceService,
        resources: &DataResourceRepo,
        results: &ResultRepo,
        versions: &VersionRepo,
    ) -> Result<Self, AnalysisError> {
        let mut entries = Vec::with_capacity(inputs.len());
        let mut active_version_id = None;
        let mut dataset_ids = Vec::new();
        for (ordinal, input) in inputs.iter().enumerate() {
            let ordinal = i32::try_from(ordinal).map_err(|error| {
                AnalysisError::new("analysis_inputs_invalid", error.to_string())
            })?;
            let resolved = match &input.source {
                AnalysisInputSource::DatasetId(dataset_id) => {
                    let source = sources.resolve(principal, *dataset_id).await.map_err(|_| {
                        AnalysisError::new("analysis_input_not_found", "资源不存在")
                    })?;
                    if source.session_id != session_id {
                        return Err(AnalysisError::new("analysis_input_not_found", "资源不存在"));
                    }
                    dataset_ids.push(*dataset_id);
                    ResolvedAnalysisInput::from_dataset(input.alias.clone(), ordinal, source)?
                }
                AnalysisInputSource::DataResourceId(resource_id) => {
                    let resource = resources
                        .find(&principal.id, resource_id, false)
                        .await
                        .map_err(|_| {
                            AnalysisError::new("analysis_input_not_found", "资源不存在")
                        })?;
                    if resource.resource.session_id != session_id {
                        return Err(AnalysisError::new("analysis_input_not_found", "资源不存在"));
                    }
                    let artifact_id =
                        resource.resource.normalized_artifact_id.ok_or_else(|| {
                            AnalysisError::new("analysis_input_not_found", "资源不存在")
                        })?;
                    let id = resource.resource.id.clone();
                    let content_hash = resource.normalized_sha256.clone();
                    ResolvedAnalysisInput {
                        snapshot: ResultAnalysisInput {
                            analysis_run_id: 0,
                            source_kind: "data_resource".into(),
                            dataset_id: None,
                            resource_id: Some(id.clone()),
                            model_version_id: None,
                            model_dataframe_id: None,
                            alias: input.alias.clone(),
                            ordinal,
                            source_name: resource.resource.display_name,
                            source_schema_json: resource.resource.schema_json,
                            source_schema_hash: resource.resource.schema_hash,
                            source_content_hash: content_hash.clone(),
                            source_producer_json: json!({
                                "kind":"data_resource",
                                "resourceId":id,
                                "contentHash":content_hash
                            })
                            .to_string(),
                        },
                        artifact_id,
                        input_key: format!("resource-{resource_id}"),
                    }
                }
                AnalysisInputSource::ActiveModelDataFrameId(dataframe_id) => {
                    let source = versions
                        .find_active_version_dataframe(&principal.id, session_id, *dataframe_id)
                        .await
                        .map_err(|error| match error {
                            VersionRepoError::Repository(_) => {
                                AnalysisError::new("analysis_input_not_found", "资源不存在")
                            }
                            _ => AnalysisError::new("analysis_input_not_found", "资源不存在"),
                        })?;
                    if active_version_id
                        .replace(source.version_id)
                        .is_some_and(|version_id| version_id != source.version_id)
                    {
                        return Err(AnalysisError::new(
                            "active_model_changed",
                            "当前激活模型已变化",
                        ));
                    }
                    ResolvedAnalysisInput::from_active_model(input.alias.clone(), ordinal, source)
                }
            };
            entries.push(resolved);
        }
        if let Some(version_id) = active_version_id {
            super::model_origin::require_matching_dataset_origins(
                results,
                &principal.id,
                session_id,
                &dataset_ids,
                version_id,
            )
            .await?;
        }
        Ok(Self { entries })
    }

    pub(super) fn snapshot_rows(&self) -> Vec<ResultAnalysisInput> {
        self.entries
            .iter()
            .map(|entry| entry.snapshot.clone())
            .collect()
    }

    pub(super) async fn stage(
        &self,
        artifacts: &ArtifactService,
        scope_id: i32,
        tree: &Path,
    ) -> Result<Vec<NamedInput>, AnalysisError> {
        let mut named_inputs = Vec::with_capacity(self.entries.len());
        for entry in &self.entries {
            let artifact = artifacts
                .find_owned(scope_id, entry.artifact_id)
                .await
                .map_err(|error| {
                    AnalysisError::new("analysis_input_not_found", error.to_string())
                })?;
            let source = artifacts
                .store()
                .resolve(&artifact.storage_path)
                .map_err(|error| {
                    AnalysisError::new("analysis_input_not_found", error.to_string())
                })?;
            let relative_path = PathBuf::from(format!("inputs/{}.parquet", entry.input_key));
            std::fs::copy(source, tree.join(&relative_path)).map_err(|error| {
                AnalysisError::new("analysis_staging_failed", error.to_string())
            })?;
            named_inputs.push(NamedInput {
                alias: entry.snapshot.alias.clone(),
                relative_path,
            });
        }
        Ok(named_inputs)
    }

    #[cfg(test)]
    fn entries(&self) -> &[ResolvedAnalysisInput] {
        &self.entries
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::application::ArtifactRef;
    use crate::application::dataframe_reader::{DataFrameColumn, DataFrameReadSource};
    use crate::application::result_sources::ResultSource;
    use crate::db::repositories::RunDataset;

    fn result_source() -> ResultSource {
        let schema_json = json!([{
            "name":"load","dtype":"Float64","nullable":false,"unit":"MW"
        }])
        .to_string();
        ResultSource {
            dataset: RunDataset {
                id: 7,
                run_id: Some(5),
                analysis_run_id: None,
                artifact_id: Some(11),
                result_name: "power".into(),
                schema_json: schema_json.clone(),
                schema_hash: "schema-hash".into(),
                row_count: 3,
                size_bytes: 20,
                created_at_ms: 1,
                deleted_at_ms: None,
                tombstone_json: None,
            },
            session_id: 3,
            artifact: ArtifactRef {
                id: 11,
                sha256: "content-hash".into(),
                size_bytes: 20,
                relative_path: "objects/content-hash".into(),
            },
            columns: vec![DataFrameColumn {
                name: "load".into(),
                dtype: "Float64".into(),
                nullable: false,
                unit: Some("MW".into()),
            }],
            read_source: DataFrameReadSource {
                resource_key: "dataset:7".into(),
                artifact_id: 11,
                schema_json,
                row_count: 3,
                content_hash: "content-hash".into(),
            },
        }
    }

    #[test]
    fn resolved_dataset_drives_snapshot_and_staging_from_one_source() {
        let resolved = ResolvedAnalysisInputs::from_entries(vec![
            ResolvedAnalysisInput::from_dataset("baseline".into(), 0, result_source()).unwrap(),
        ]);

        let rows = resolved.snapshot_rows();
        assert_eq!(rows[0].dataset_id, Some(7));
        assert_eq!(rows[0].source_content_hash, "content-hash");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&rows[0].source_producer_json).unwrap(),
            json!({"kind":"run","runId":"5"})
        );
        assert_eq!(resolved.entries()[0].artifact_id(), 11);
        assert_eq!(resolved.entries()[0].input_key(), "dataset-7");
    }
}
