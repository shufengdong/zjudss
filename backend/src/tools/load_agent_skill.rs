use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};
use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::Arc;

#[derive(Deserialize)]
pub struct Args {
    pub skill_id: String,
}
pub struct LoadAgentSkillTool {
    services: Arc<ToolServices>,
}
impl LoadAgentSkillTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for LoadAgentSkillTool {
    const NAME: &'static str = "load_agent_skill";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "按稳定 skill ID 加载一个版本化领域技能；脚本生成前必须加载对应技能。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(json!({"skill_id":{"type":"string"}}), &["skill_id"])
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let skill = self
            .services
            .agent_knowledge
            .skill(&args.skill_id)
            .ok_or_else(|| DomainToolError::new("skill_not_found", "未知 skill ID"))?;
        if skill.content.chars().count() > 32_000 {
            return Err(DomainToolError::new(
                "skill_too_large",
                "skill 内容超过单次工具输出上限",
            ));
        }
        self.services
            .loaded_skills
            .lock()
            .await
            .insert(args.skill_id);
        Ok(
            json!({"id":skill.metadata.id,"version":skill.metadata.version,"sha256":skill.metadata.sha256,"content":skill.content}),
        )
    }
}
