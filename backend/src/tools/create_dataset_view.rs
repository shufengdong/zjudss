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
    dataset_id: Option<i32>,
    resource_id: Option<String>,
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
        "为已有结果数据集或普通数据资源保存服务端验证的折线、柱状、散点、热力、箱线或直方图物化视图；不会重新运行 tensoreval。".into()
    }
    fn parameters(&self) -> Value {
        let mut schema = object_schema(
            json!({"datasetId":{"type":["integer","null"]},"resourceId":{"type":["string","null"]},"name":{"type":"string"},"spec":{"type":"object"}}),
            &["name", "spec"],
        );
        schema["oneOf"] = json!([
            {"required":["datasetId"]},
            {"required":["resourceId"]}
        ]);
        schema
    }
    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_destructive(true)
    }
    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        let view = match (args.dataset_id, args.resource_id) {
            (Some(dataset_id), None) => {
                self.services
                    .analysis
                    .create_view(&self.services.principal, dataset_id, args.name, args.spec)
                    .await?
            }
            (None, Some(resource_id)) => {
                self.services
                    .analysis
                    .create_resource_view(
                        &self.services.principal,
                        &resource_id,
                        args.name,
                        args.spec,
                    )
                    .await?
            }
            _ => {
                return Err(DomainToolError::new(
                    "visualization_source_invalid",
                    "必须且只能提供 datasetId 或 resourceId",
                ));
            }
        };
        Ok(
            json!({"viewId":view.id.to_string(),"datasetId":view.source_dataset_id.to_string(),"viewDatasetId":view.view_dataset_id.to_string(),"kind":view.kind}),
        )
    }
}
