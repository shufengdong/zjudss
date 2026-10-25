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
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub name: String,
    pub content: String,
    #[serde(default)]
    pub study_plan_id: Option<String>,
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
        "创建一个供用户检查、复制并通过脚本卡片按钮运行的 RSS 脚本；本工具绝不运行脚本。调用前必须加载 rss-generation skill；调用受管研究函数时必须传入 plan_study 本次返回的 studyPlanId。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"name":{"type":"string"},"content":{"type":"string"},"studyPlanId":{"type":"string","pattern":"^study-plan:[0-9a-f]{64}$"}}),
            &["name", "content"],
        )
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("rss-generation").await?;
        let study_plan = match args.study_plan_id {
            Some(id) => Some(
                self.services
                    .ready_study_plans
                    .lock()
                    .await
                    .get(&id)
                    .ok_or_else(|| {
                        DomainToolError::new(
                            "study_plan_not_found",
                            "studyPlanId 不属于当前请求或已经失效",
                        )
                    })?,
            ),
            None => None,
        };
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
                    study_plan,
                },
            )
            .await?;
        Ok(serde_json::to_value(summary).unwrap_or_default())
    }
}
