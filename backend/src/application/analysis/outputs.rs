use std::collections::HashSet;

use crate::adapters::EngineDatasetOutput;
use crate::application::{ArtifactMetadata, ArtifactService, now_ms};
use crate::db::models::NewResultDataset;
use crate::storage::{ParquetInspector, StageLimits};

use super::AnalysisError;

pub(super) fn validate_output_names(
    outputs: &[EngineDatasetOutput],
    expected: &[String],
) -> Result<(), AnalysisError> {
    let names = outputs
        .iter()
        .map(|dataset| dataset.name.as_str())
        .collect::<HashSet<_>>();
    if names.len() != expected.len() || expected.iter().any(|name| !names.contains(name.as_str())) {
        return Err(AnalysisError::new(
            "analysis_output_mismatch",
            "实际输出与声明输出不一致",
        ));
    }
    Ok(())
}

pub(super) struct CommittedAnalysisOutputs {
    artifact_ids: Vec<i32>,
    drafts: Vec<NewResultDataset>,
}

impl CommittedAnalysisOutputs {
    pub(super) fn drafts(&self) -> Vec<NewResultDataset> {
        self.drafts.clone()
    }

    pub(super) async fn release(&self, artifacts: &ArtifactService, scope_id: i32) {
        for artifact_id in &self.artifact_ids {
            let _ = artifacts.release(scope_id, *artifact_id).await;
        }
    }
}

pub(super) async fn collect_outputs(
    outputs: Vec<EngineDatasetOutput>,
    analysis_run_id: i32,
    scope_id: i32,
    artifacts: &ArtifactService,
) -> Result<CommittedAnalysisOutputs, AnalysisError> {
    let mut artifact_ids = Vec::new();
    let mut drafts = Vec::new();
    for output in outputs {
        let metadata = match ParquetInspector::metadata(&output.path) {
            Ok(metadata) => metadata,
            Err(error) => {
                release_ids(artifacts, scope_id, &artifact_ids).await;
                return Err(AnalysisError::new(
                    "analysis_output_invalid",
                    error.to_string(),
                ));
            }
        };
        if !metadata.snappy {
            release_ids(artifacts, scope_id, &artifact_ids).await;
            return Err(AnalysisError::new(
                "analysis_output_invalid",
                "输出必须为 Snappy Parquet",
            ));
        }
        let file = match std::fs::File::open(&output.path) {
            Ok(file) => file,
            Err(error) => {
                release_ids(artifacts, scope_id, &artifact_ids).await;
                return Err(AnalysisError::new(
                    "analysis_output_invalid",
                    error.to_string(),
                ));
            }
        };
        let staged = match artifacts.store().stage(
            file,
            StageLimits {
                max_bytes: 500 * 1024 * 1024,
            },
        ) {
            Ok(staged) => staged,
            Err(error) => {
                release_ids(artifacts, scope_id, &artifact_ids).await;
                return Err(AnalysisError::new(
                    "analysis_output_invalid",
                    error.to_string(),
                ));
            }
        };
        let artifact = match artifacts
            .commit(scope_id, staged, ArtifactMetadata::parquet())
            .await
        {
            Ok(artifact) => artifact,
            Err(error) => {
                release_ids(artifacts, scope_id, &artifact_ids).await;
                return Err(AnalysisError::new(
                    "analysis_output_commit_failed",
                    error.to_string(),
                ));
            }
        };
        artifact_ids.push(artifact.id);
        let (schema_json, schema_hash) =
            crate::application::datasets::schema_json_and_hash(&metadata);
        drafts.push(NewResultDataset {
            run_id: None,
            analysis_run_id: Some(analysis_run_id),
            artifact_id: Some(artifact.id),
            result_name: output.name,
            schema_json,
            schema_hash,
            row_count: metadata.row_count as i64,
            size_bytes: metadata.size_bytes as i64,
            created_at_ms: now_ms(),
        });
    }
    Ok(CommittedAnalysisOutputs {
        artifact_ids,
        drafts,
    })
}

async fn release_ids(artifacts: &ArtifactService, scope_id: i32, ids: &[i32]) {
    for artifact_id in ids {
        let _ = artifacts.release(scope_id, *artifact_id).await;
    }
}
