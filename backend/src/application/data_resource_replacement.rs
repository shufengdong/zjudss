use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::auth::CurrentUser;
use crate::catalog::DataFrameContractKind;
use crate::db::repositories::{StructuredPreviewConfirmation, StructuredPreviewInput};
use crate::storage::StageLimits;

use super::data_resource_parquet::{dtype_label, struct_parquet};
use super::{
    DataResourceError, DataResourceService, DataResourceView, read_source, stable_resource_id,
};
use crate::application::opaque_token::{generate_token, hash_token, validate_presented_token};
use crate::application::upload_confirmation::ReplacementAction;
use crate::application::{ArtifactError, ArtifactMetadata, now_ms};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructuredReplacementPreview {
    pub preview_token: String,
    pub resource_id: String,
    pub expected_content_hash: String,
    pub schema_ref: String,
    pub differences: Vec<Value>,
    pub expires_at_ms: i64,
}

impl DataResourceService {
    pub async fn preview_structured_replacement(
        &self,
        user: &CurrentUser,
        id: &str,
        expected_content_hash: &str,
        schema_ref: &str,
        values: Value,
    ) -> Result<StructuredReplacementPreview, DataResourceError> {
        let current = self.repo.find(&user.id, id, false).await?;
        if current.normalized_sha256 != expected_content_hash {
            return Err(DataResourceError::Stale);
        }
        if current.resource.declared_schema_ref.as_deref() != Some(schema_ref) {
            return Err(Self::invalid(
                "structured_schema_mismatch",
                "资源声明的 Struct schema 与请求不一致",
            ));
        }
        let contract = self
            .snapshot
            .schemas
            .resolve(schema_ref)
            .map_err(|error| Self::invalid("structured_schema_mismatch", &error.to_string()))?;
        if contract.kind != DataFrameContractKind::Struct {
            return Err(Self::invalid(
                "structured_schema_mismatch",
                "schema 不是 Struct",
            ));
        }
        let object = values
            .as_object()
            .ok_or_else(|| Self::invalid("structured_values_invalid", "values 必须是对象"))?;
        let old_page = self
            .reader
            .page(user, &read_source(&current), Some(1), None, &[])
            .await?;
        let parquet = struct_parquet(contract, object)?;
        let staged = self
            .artifacts
            .store()
            .stage(std::io::Cursor::new(parquet), StageLimits::default())
            .map_err(ArtifactError::from)?;
        let artifact = self
            .artifacts
            .commit(user.storage_scope_id, staged, ArtifactMetadata::parquet())
            .await?;
        let columns = contract
            .fields
            .iter()
            .map(
                |field| crate::application::dataframe_reader::DataFrameColumn {
                    name: field.name.clone(),
                    dtype: dtype_label(&field.dtype),
                    nullable: field.nullable,
                    unit: Some(field.unit.0.clone()),
                },
            )
            .collect::<Vec<_>>();
        let schema_json = serde_json::to_string(&columns)
            .map_err(|error| Self::invalid("structured_values_invalid", &error.to_string()))?;
        let differences = contract
            .fields
            .iter()
            .enumerate()
            .filter_map(|(index, field)| {
                let before = old_page
                    .rows
                    .first()
                    .and_then(|row| row.get(index))
                    .cloned()
                    .unwrap_or(Value::Null);
                let after = object.get(&field.name).cloned().unwrap_or(Value::Null);
                (before != after)
                    .then(|| serde_json::json!({"field":field.name,"before":before,"after":after}))
            })
            .collect::<Vec<_>>();
        let now = now_ms();
        let pending_id = stable_resource_id(
            current.resource.session_id,
            &format!("structured:{id}:{now}:{}", artifact.sha256),
        );
        let preview_token = generate_token();
        let token_hash = hash_token(&preview_token);
        let impact_json = serde_json::to_string(&differences).unwrap();
        let impact_hash = hex::encode(Sha256::digest(impact_json.as_bytes()));
        let expires = now + 10 * 60 * 1000;
        let input = StructuredPreviewInput {
            token_hash,
            user_id: user.id.0.clone(),
            old_resource_id: id.into(),
            pending_artifact_id: artifact.id,
            pending_resource_id: pending_id,
            expected_content_hash: expected_content_hash.into(),
            schema_ref: schema_ref.into(),
            schema_json,
            schema_hash: contract.schema_hash.clone(),
            values_json: values.to_string(),
            display_name: current.resource.display_name.clone(),
            proposed_alias: current.resource.alias.clone(),
            impact_hash,
            impact_json,
            expires_at_ms: expires,
            created_at_ms: now,
        };
        match self.repo.store_structured_preview(input).await {
            Ok(()) => {}
            Err(error) => {
                let _ = self
                    .artifacts
                    .release(user.storage_scope_id, artifact.id)
                    .await;
                return Err(error.into());
            }
        }
        Ok(StructuredReplacementPreview {
            preview_token,
            resource_id: id.into(),
            expected_content_hash: expected_content_hash.into(),
            schema_ref: schema_ref.into(),
            differences,
            expires_at_ms: expires,
        })
    }

    pub async fn confirm_structured_replacement(
        &self,
        user: &CurrentUser,
        preview_token: &str,
        action: ReplacementAction,
    ) -> Result<Option<DataResourceView>, DataResourceError> {
        validate_presented_token(preview_token)
            .map_err(|_| Self::invalid("replacement_preview_invalid", "替换预览不存在或已过期"))?;
        let token_hash = hash_token(preview_token);
        let confirmation = self
            .repo
            .confirm_structured_preview(&user.id, token_hash, action.as_str().into(), now_ms())
            .await?;
        match confirmation {
            StructuredPreviewConfirmation::Published(resource) => {
                Ok(Some(self.get(user, &resource.id).await?))
            }
            StructuredPreviewConfirmation::Cancelled => Ok(None),
            StructuredPreviewConfirmation::Rejected { code } => Err(Self::invalid(
                code,
                if code == "resource_stale" {
                    "原资源已发生变化，请重新生成预览"
                } else {
                    "替换预览不存在或已过期"
                },
            )),
        }
    }

    pub async fn cleanup_expired_structured_previews(&self) -> Result<usize, DataResourceError> {
        self.repo
            .take_expired_structured_previews(now_ms())
            .await
            .map_err(Into::into)
    }
}
