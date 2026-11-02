use std::sync::Arc;

use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{SelectStudyResultsRequest, StudyConfigCard};
use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub configuration_id: String,
    pub expected_revision: i64,
    pub results: Vec<String>,
}

pub struct UpdateStudyResultsTool {
    services: Arc<ToolServices>,
}

impl UpdateStudyResultsTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

impl Tool for UpdateStudyResultsTool {
    const NAME: &'static str = "update_study_results";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;

    fn description(&self) -> String {
        "更新当前会话中尚未确认的研究配置结果选择，并刷新研究配置卡；不会确认配置、生成脚本或启动运行。调用前加载 study-planning skill。".into()
    }

    fn parameters(&self) -> Value {
        object_schema(
            json!({
                "configurationId": {
                    "type": "string",
                    "pattern": "^[1-9][0-9]*$",
                    "maxLength": 20
                },
                "expectedRevision": {"type": "integer", "minimum": 1},
                "results": {
                    "type": "array",
                    "minItems": 1,
                    "maxItems": 128,
                    "uniqueItems": true,
                    "items": {"type": "string", "minLength": 1, "maxLength": 256}
                }
            }),
            &["configurationId", "expectedRevision", "results"],
        )
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_destructive(true)
    }

    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("study-planning").await?;
        let configuration_id = args
            .configuration_id
            .parse::<i32>()
            .ok()
            .filter(|id| *id > 0)
            .ok_or_else(|| DomainToolError::new("not_found", "研究配置不存在"))?;
        let current = self
            .services
            .study_configurations
            .get(&self.services.principal, configuration_id)
            .await?;
        if current.session_id != self.services.session_id {
            return Err(DomainToolError::new("not_found", "研究配置不存在"));
        }

        let configuration = self
            .services
            .study_configurations
            .select_results(
                &self.services.principal,
                configuration_id,
                SelectStudyResultsRequest {
                    expected_revision: args.expected_revision,
                    results: args.results,
                },
            )
            .await?;
        let value = serde_json::to_value(&configuration)
            .map_err(|error| DomainToolError::new("serialization_error", error.to_string()))?;
        self.services
            .writer
            .study_config(StudyConfigCard {
                configuration_id: configuration.id.to_string(),
                revision: configuration.revision,
                state: value["status"]
                    .as_str()
                    .unwrap_or("collecting_inputs")
                    .to_string(),
                action_card: value.clone(),
            })
            .await
            .map_err(|error| {
                DomainToolError::new("study_config_message_failed", error.to_string())
            })?;
        Ok(value)
    }
}
