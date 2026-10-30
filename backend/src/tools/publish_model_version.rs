use std::sync::Arc;

use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{PublishDraftRequest, VersionError};
use crate::db::models::DraftRef;
use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub expected_draft: DraftRef,
    #[serde(default)]
    pub description: Option<String>,
}

pub struct PublishModelVersionTool {
    services: Arc<ToolServices>,
}

impl PublishModelVersionTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

impl Tool for PublishModelVersionTool {
    const NAME: &'static str = "publish_model_version";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;

    fn description(&self) -> String {
        "仅在用户明确要求发布当前模型版本时，发布并自动激活 inspect_workspace 返回的精确草稿 revision；不得用于普通修改、导入或研究请求。".into()
    }

    fn parameters(&self) -> Value {
        object_schema(
            json!({
                "expectedDraft": {
                    "type": "object",
                    "properties": {
                        "id": {"type": "integer"},
                        "revision": {"type": "integer"}
                    },
                    "required": ["id", "revision"],
                    "additionalProperties": false
                },
                "description": {
                    "anyOf": [{"type": "string"}, {"type": "null"}]
                }
            }),
            &["expectedDraft"],
        )
    }

    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("model-intake").await?;
        let published = self
            .services
            .versions
            .publish(
                &self.services.principal,
                self.services.session_id,
                PublishDraftRequest {
                    expected_draft_id: args.expected_draft.id,
                    expected_draft_revision: args.expected_draft.revision,
                    client_publish_id: format!(
                        "agent-publish:{}:{}:{}",
                        self.services.request_message_id,
                        args.expected_draft.id,
                        args.expected_draft.revision
                    ),
                    description: args.description,
                },
            )
            .await
            .map_err(map_version_error)?;
        let version = published.version.version;
        Ok(json!({
            "versionId": version.id.to_string(),
            "description": version.description,
            "contentHash": version.content_hash,
            "active": true,
            "deduplicated": published.deduplicated,
            "replayed": published.replayed,
        }))
    }
}

fn map_version_error(error: VersionError) -> DomainToolError {
    match error {
        VersionError::DraftRevisionConflict => {
            DomainToolError::new("draft_revision_conflict", error.to_string())
        }
        VersionError::DraftRequired | VersionError::EmptyDraft => {
            DomainToolError::new("workspace_error", error.to_string())
        }
        _ => DomainToolError::new("workspace_error", error.to_string()),
    }
}
