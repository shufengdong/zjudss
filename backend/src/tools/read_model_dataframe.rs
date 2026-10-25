use std::collections::BTreeMap;
use std::sync::Arc;

use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::{ModelDataFrameQuery, ModelReadScope};
use crate::db::models::DraftRef;
use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    pub scope: ModelReadScope,
    pub table_name: String,
    #[serde(default)]
    pub columns: Vec<String>,
    #[serde(default)]
    pub equals: BTreeMap<String, Value>,
    pub limit: Option<usize>,
    pub expected_draft: Option<DraftRef>,
}

pub struct ReadModelDataFrameTool {
    services: Arc<ToolServices>,
}

impl ReadModelDataFrameTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

impl Tool for ReadModelDataFrameTool {
    const NAME: &'static str = "read_model_dataframe";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;

    fn description(&self) -> String {
        "按当前会话的 active 或精确 draft revision 读取模型 DataFrame 行；只支持受控列投影和 AND 等值条件。调用前必须加载 model-intake。"
            .into()
    }

    fn parameters(&self) -> Value {
        object_schema(
            json!({
                "scope":{"type":"string","enum":["active","draft"]},
                "tableName":{"type":"string"},
                "columns":{"type":"array","items":{"type":"string"},"maxItems":32},
                "equals":{"type":"object","maxProperties":8,"additionalProperties":{"type":["string","boolean","number"]}},
                "limit":{"type":"integer","minimum":1,"maximum":100,"default":20},
                "expectedDraft":{"anyOf":[
                    {"type":"object","properties":{"id":{"type":"integer"},"revision":{"type":"integer"}},"required":["id","revision"],"additionalProperties":false},
                    {"type":"null"}
                ]}
            }),
            &["scope", "tableName", "columns", "equals", "expectedDraft"],
        )
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("model-intake").await?;
        let result = self
            .services
            .model_dataframes
            .query_workspace(
                &self.services.principal,
                self.services.session_id,
                ModelDataFrameQuery {
                    scope: args.scope,
                    table_name: args.table_name,
                    columns: args.columns,
                    equals: args.equals,
                    limit: args.limit.unwrap_or(20),
                    expected_draft: args.expected_draft,
                },
            )
            .await
            .map_err(|error| DomainToolError::new(error.code(), error.to_string()))?;
        serde_json::to_value(result)
            .map_err(|error| DomainToolError::new("dataframe_read_failed", error.to_string()))
    }
}
