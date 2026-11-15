use std::sync::Arc;

use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::application::DatasetError;
use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};

const MAX_SUCCESS_BYTES: usize = 64 * 1024;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Args {
    pub dataset_id: i32,
    #[serde(default)]
    pub columns: Vec<String>,
    pub limit: Option<usize>,
    pub cursor: Option<String>,
}

pub struct ReadResultDatasetTool {
    services: Arc<ToolServices>,
}

impl ReadResultDatasetTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

impl Tool for ReadResultDatasetTool {
    const NAME: &'static str = "read_result_dataset";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;

    fn description(&self) -> String {
        "读取当前会话已有结果 DataFrame 的元数据和有界分页内容；最多 100 行、32 列。调用前必须加载 result-analysis。".into()
    }

    fn parameters(&self) -> Value {
        object_schema(
            json!({
                "datasetId":{"type":"integer"},
                "columns":{"type":"array","items":{"type":"string"},"maxItems":32},
                "limit":{"type":"integer","minimum":1,"maximum":100,"default":20},
                "cursor":{"type":["string","null"]}
            }),
            &["datasetId"],
        )
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("result-analysis").await?;
        if args.columns.len() > 32 || args.limit.is_some_and(|limit| !(1..=100).contains(&limit)) {
            return Err(DomainToolError::new(
                "invalid_dataset_page",
                "结果读取限制为 1..=100 行和最多 32 列",
            ));
        }
        let session_id = self
            .services
            .datasets
            .session_id(&self.services.principal, args.dataset_id)
            .await
            .map_err(map_dataset_error)?;
        if session_id != self.services.session_id {
            return Err(DomainToolError::new("dataset_not_found", "资源不存在"));
        }
        let metadata = self
            .services
            .datasets
            .metadata(&self.services.principal, args.dataset_id)
            .await
            .map_err(map_dataset_error)?;
        let page = self
            .services
            .datasets
            .page(
                &self.services.principal,
                args.dataset_id,
                args.limit,
                args.cursor.as_deref(),
                &args.columns,
            )
            .await
            .map_err(map_dataset_error)?;
        let output = json!({"metadata": metadata, "page": page});
        let output_bytes = serde_json::to_vec(&output).map_err(|error| {
            DomainToolError::new(
                "dataset_read_failed",
                format!("结果响应序列化失败: {error}"),
            )
        })?;
        if output_bytes.len() > MAX_SUCCESS_BYTES {
            return Err(DomainToolError::new(
                "dataset_read_failed",
                "结果读取响应超过 64 KiB，请减少列、行数或缩小单元格内容后重试",
            ));
        }
        Ok(output)
    }
}

fn map_dataset_error(error: DatasetError) -> DomainToolError {
    let code = match &error {
        DatasetError::NotFound => "dataset_not_found",
        DatasetError::InvalidCursor | DatasetError::InvalidPage(_) => "invalid_dataset_page",
        _ => "dataset_read_failed",
    };
    DomainToolError::new(code, error.to_string())
}
