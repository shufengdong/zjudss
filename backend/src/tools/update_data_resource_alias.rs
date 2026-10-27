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
    pub resource_id: String,
    pub alias: String,
}
pub struct UpdateDataResourceAliasTool {
    services: Arc<ToolServices>,
}
impl UpdateDataResourceAliasTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for UpdateDataResourceAliasTool {
    const NAME: &'static str = "update_data_resource_alias";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "按用户明确要求修改当前会话普通数据资源的 RSS alias；服务端校验标识符、保留字和会话唯一性。"
            .into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"resourceId":{"type":"string"},"alias":{"type":"string","minLength":1,"maxLength":128}}),
            &["resourceId", "alias"],
        )
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let current = self
            .services
            .data_resources
            .get(&self.services.principal, &args.resource_id)
            .await
            .map_err(|e| DomainToolError::new(e.code(), e.to_string()))?;
        if current.session_id != self.services.session_id.to_string() {
            return Err(DomainToolError::new("not_found", "资源不存在"));
        }
        let updated = self
            .services
            .data_resources
            .rename_alias(&self.services.principal, &args.resource_id, &args.alias)
            .await
            .map_err(|e| DomainToolError::new(e.code(), e.to_string()))?;
        serde_json::to_value(updated)
            .map_err(|e| DomainToolError::new("serialization_error", e.to_string()))
    }
}
