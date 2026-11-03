use std::sync::Arc;

use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{PrepareStudyRequest, StudyConfigCard};
use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub facade_id: String,
    pub intent: Value,
    pub derived_from_workflow_id: Option<i32>,
    pub explanation: String,
}

pub struct PrepareStudyTool {
    services: Arc<ToolServices>,
}

impl PrepareStudyTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

impl Tool for PrepareStudyTool {
    const NAME: &'static str = "prepare_study";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;

    fn description(&self) -> String {
        "用已搜索并选定的目录门面和模型生成的研究方案说明创建持久化研究配置卡。只准备输入、Struct 参数和结果选择，不生成或运行脚本。调用前加载 study-planning skill。".into()
    }

    fn parameters(&self) -> Value {
        object_schema(
            json!({
                "facadeId":{"type":"string","minLength":1,"maxLength":256},
                "intent":{"type":"object"},
                "derivedFromWorkflowId":{"type":["integer","null"],"minimum":1},
                "explanation":{"type":"string","minLength":1,"maxLength":4000}
            }),
            &["facadeId", "intent", "explanation"],
        )
    }

    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("study-planning").await?;
        let explanation = args.explanation.trim().to_string();
        if explanation.is_empty() || explanation.chars().count() > 4_000 {
            return Err(DomainToolError::new(
                "invalid_study_explanation",
                "研究方案说明必须为 1 到 4000 个字符",
            ));
        }
        let configuration = self
            .services
            .study_configurations
            .prepare(
                &self.services.principal,
                self.services.session_id,
                PrepareStudyRequest {
                    facade_id: args.facade_id,
                    intent: args.intent,
                    derived_from_workflow_id: args.derived_from_workflow_id,
                },
            )
            .await?;
        let value = serde_json::to_value(&configuration)
            .map_err(|error| DomainToolError::new("serialization_error", error.to_string()))?;
        self.services
            .writer
            .assistant(&format!(
                "{explanation}\n\n已创建“{}”研究配置卡（ID: {}）。请在卡片中检查并确认输入、参数和结果；只有点击“确认并生成脚本”后才会开始执行。",
                configuration.facade.title, configuration.id,
            ))
            .await
            .map_err(|error| {
                DomainToolError::new("study_config_message_failed", error.to_string())
            })?;
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
