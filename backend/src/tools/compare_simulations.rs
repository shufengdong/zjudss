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
#[serde(rename_all = "camelCase")]
pub struct Args {
    baseline_run_id: i32,
    candidate_run_id: i32,
}
pub struct CompareSimulationsTool {
    services: Arc<ToolServices>,
}
impl CompareSimulationsTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for CompareSimulationsTool {
    const NAME: &'static str = "compare_simulations";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "比较当前会话中两个已完成运行；服务端选择兼容数据集、稳定键和单位并生成可追溯分析。调用前必须加载 comparison。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"baselineRunId":{"type":"integer"},"candidateRunId":{"type":"integer"}}),
            &["baselineRunId", "candidateRunId"],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_destructive(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("comparison").await?;
        let skill = self.services.agent_knowledge.skill("comparison").unwrap();
        let c=self.services.comparisons.compare(&self.services.principal,self.services.session_id,args.baseline_run_id,args.candidate_run_id,Some(self.services.request_message_id),vec![json!({"id":skill.metadata.id,"version":skill.metadata.version,"sha256":skill.metadata.sha256})]).await?;
        Ok(json!({"comparisonId":c.id.to_string(),"analysisRunId":c.analysis_run_id.to_string()}))
    }
}
