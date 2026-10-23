use crate::application::{CreateScriptRequest, ScriptSource};
use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct Args {
    pub name: String,
    pub content: String,
}
pub struct CreateScriptTool {
    services: Arc<ToolServices>,
}
impl CreateScriptTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for CreateScriptTool {
    const NAME: &'static str = "create_script";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "创建一个供用户检查、复制并通过脚本卡片按钮运行的 RSS 脚本；本工具绝不运行脚本。调用前必须加载 rss-generation skill。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"name":{"type":"string"},"content":{"type":"string"}}),
            &["name", "content"],
        )
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("rss-generation").await?;
        let skill_refs = vec![json!({"id":"rss-generation"})];
        let summary = self
            .services
            .scripts
            .create(
                &self.services.principal,
                self.services.session_id,
                CreateScriptRequest {
                    name: args.name,
                    content: args.content,
                    source: ScriptSource::Agent,
                    agent_bundle_version: Some(
                        self.services
                            .agent_knowledge
                            .manifest
                            .agent_bundle_version
                            .clone(),
                    ),
                    agent_skill_refs: skill_refs,
                },
            )
            .await?;
        Ok(serde_json::to_value(summary).unwrap_or_default())
    }
}
