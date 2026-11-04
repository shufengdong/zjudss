use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::application::now_ms;
use crate::application::opaque_token::{generate_token, hash_token, validate_presented_token};
use crate::auth::CurrentUser;
use crate::db::repositories::{DeletionImpactRow, DeletionPreviewInput, RepoError};

use super::{DataResourceError, DataResourceService};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DataResourceDeletionPreview {
    pub impact_token: String,
    pub resource_id: String,
    pub display_name: String,
    pub alias: String,
    pub size_bytes: i64,
    pub pending_configuration_count: i64,
    pub frozen_reference_count: i64,
    pub historical_reference_count: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedDataResource {
    pub resource_id: String,
    pub tombstone: Value,
}

impl DataResourceService {
    pub async fn preview_deletion(
        &self,
        user: &CurrentUser,
        id: &str,
    ) -> Result<DataResourceDeletionPreview, DataResourceError> {
        let impact = self.repo.deletion_impact(&user.id, id).await?;
        let value = impact_value(&impact);
        let impact_json = serde_json::to_string(&value).expect("impact serializable");
        let impact_hash = hex::encode(Sha256::digest(impact_json.as_bytes()));
        let token = generate_token();
        let token_hash = hash_token(&token);
        let expires = now_ms() + 10 * 60 * 1000;
        self.repo
            .store_deletion_preview(
                &user.id,
                DeletionPreviewInput {
                    resource_id: id.into(),
                    token_hash,
                    impact_hash,
                    impact_json,
                    expires_at_ms: expires,
                    created_at_ms: now_ms(),
                },
            )
            .await?;
        Ok(DataResourceDeletionPreview {
            impact_token: token,
            resource_id: id.into(),
            display_name: impact.resource.display_name,
            alias: impact.resource.alias,
            size_bytes: impact.normalized_size_bytes,
            pending_configuration_count: impact.pending_configuration_count,
            frozen_reference_count: impact.frozen_reference_count,
            historical_reference_count: impact.historical_reference_count,
            expires_at_ms: expires,
        })
    }

    pub async fn delete(
        &self,
        user: &CurrentUser,
        id: &str,
        impact_token: &str,
    ) -> Result<DeletedDataResource, DataResourceError> {
        let impact = match self.repo.deletion_impact(&user.id, id).await {
            Ok(value) => value,
            Err(RepoError::NotFound) => {
                let row = self.repo.find_record(&user.id, id).await?;
                if row.status == "deleted" {
                    return Ok(DeletedDataResource {
                        resource_id: id.into(),
                        tombstone: row
                            .tombstone_json
                            .and_then(|value| serde_json::from_str(&value).ok())
                            .unwrap_or_else(|| serde_json::json!({"id":id})),
                    });
                }
                return Err(DataResourceError::NotFound);
            }
            Err(error) => return Err(error.into()),
        };
        let impact_hash = hex::encode(Sha256::digest(
            serde_json::to_string(&impact_value(&impact))
                .unwrap()
                .as_bytes(),
        ));
        validate_presented_token(impact_token).map_err(|_| {
            Self::invalid(
                "invalid_deletion_impact_token",
                "删除影响确认凭据无效或已过期",
            )
        })?;
        let token_hash = hash_token(impact_token);
        let deleted = self
            .repo
            .delete(&user.id, id, token_hash, impact_hash, now_ms())
            .await?;
        Ok(DeletedDataResource {
            resource_id: id.into(),
            tombstone: serde_json::json!({"id":id,"alias":deleted.resource.alias,"schemaHash":deleted.resource.schema_hash}),
        })
    }
}

fn impact_value(impact: &DeletionImpactRow) -> Value {
    serde_json::json!({"resourceId":impact.resource.id,"schemaHash":impact.resource.schema_hash,"pendingConfigurationCount":impact.pending_configuration_count,"frozenReferenceCount":impact.frozen_reference_count,"historicalReferenceCount":impact.historical_reference_count})
}
