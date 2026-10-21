//! `inspect_model` 工具：调用适配器的 `SimEngine::inspect()` 做模型检查。
//!
//! 检查结论直接作为工具返回值（即 tool-call 的 summary），不再硬编码 `"model ok"`。

use std::sync::Arc;

use rig::tool::{Tool, ToolContext};

use crate::adapters::AdapterError;
use crate::tools::ToolServices;

/// 模型检查工具的参数。
#[derive(serde::Deserialize)]
pub struct InspectModelArgs {
    /// 模型文件路径（.dss）。
    pub model_path: String,
}

/// 有状态工具实例：持有绑定到当前 session 的共享服务。
pub struct InspectModelTool {
    services: Arc<ToolServices>,
}

impl InspectModelTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

impl Tool for InspectModelTool {
    const NAME: &'static str = "inspect_model";
    type Args = InspectModelArgs;
    type Output = String;
    type Error = AdapterError;

    fn description(&self) -> String {
        "检查配电网仿真模型文件（.dss）是否可解析，返回模型结构摘要与检查结论。".to_string()
    }

    fn parameters(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "model_path": {
                    "type": "string",
                    "description": "配电网模型文件路径（.dss）"
                }
            },
            "required": ["model_path"]
        })
    }

    async fn call(
        &self,
        _context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        // 只走适配器（AGENTS.md 硬约束）。
        let outcome = self.services.engine.inspect(&args.model_path).await?;
        Ok(outcome.summary)
    }
}
