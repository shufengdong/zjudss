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
    dataset_id: i32,
    name: String,
    spec: Value,
}
pub struct CreateDatasetViewTool {
    services: Arc<ToolServices>,
}
impl CreateDatasetViewTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}
impl Tool for CreateDatasetViewTool {
    const NAME: &'static str = "create_dataset_view";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;
    fn description(&self) -> String {
        "为已有数据集保存服务端验证的折线、柱状、散点、热力、箱线或直方图视图；不会重新运行 tensoreval。".into()
    }
    fn parameters(&self) -> Value {
        object_schema(
            json!({"datasetId":{"type":"integer"},"name":{"type":"string"},"spec":{"type":"object"}}),
            &["datasetId", "name", "spec"],
        )
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_destructive(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let view = self
            .services
            .analysis
            .create_view(
                &self.services.principal,
                args.dataset_id,
                args.name,
                args.spec,
            )
            .await?;
        Ok(
            json!({"viewId":view.id.to_string(),"datasetId":view.source_dataset_id.to_string(),"viewDatasetId":view.view_dataset_id.to_string(),"kind":view.kind}),
        )
    }
}
