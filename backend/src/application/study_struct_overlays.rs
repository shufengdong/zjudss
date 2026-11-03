use serde_json::Value;

use crate::catalog::DataFrameSchemaContract;
use crate::storage::StageLimits;

use super::super::{ArtifactError, ArtifactMetadata, ArtifactService};
use super::DataResourceError;
use super::struct_parquet;

#[derive(Debug, Clone)]
pub(crate) struct StoredStructOverlay {
    pub artifact_id: i32,
    pub content_hash: String,
    pub owns_artifact_reference: bool,
}

pub(crate) async fn materialize_struct_overlay(
    artifacts: &ArtifactService,
    storage_scope_id: i32,
    contract: &DataFrameSchemaContract,
    values: &serde_json::Map<String, Value>,
    reusable_source: Option<(i32, &str)>,
) -> Result<StoredStructOverlay, DataResourceError> {
    let parquet = struct_parquet(contract, values)?;
    let staged = artifacts
        .store()
        .stage(std::io::Cursor::new(parquet), StageLimits::default())
        .map_err(ArtifactError::from)?;
    if let Some((artifact_id, content_hash)) = reusable_source
        && staged.sha256 == content_hash
    {
        return Ok(StoredStructOverlay {
            artifact_id,
            content_hash: content_hash.to_string(),
            owns_artifact_reference: false,
        });
    }
    let artifact = artifacts
        .commit(storage_scope_id, staged, ArtifactMetadata::parquet())
        .await?;
    Ok(StoredStructOverlay {
        artifact_id: artifact.id,
        content_hash: artifact.sha256,
        owns_artifact_reference: true,
    })
}
