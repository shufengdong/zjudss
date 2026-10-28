use std::sync::Arc;

use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::tools::{
    ToolServices,
    support::{DomainToolError, object_schema},
};

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Args {
    #[serde(default)]
    pub query: String,
    pub explicit_function: Option<String>,
}

pub struct SearchStudyFacadesTool {
    services: Arc<ToolServices>,
}

impl SearchStudyFacadesTool {
    pub fn new(services: Arc<ToolServices>) -> Self {
        Self { services }
    }
}

impl Tool for SearchStudyFacadesTool {
    const NAME: &'static str = "search_study_facades";
    type Args = Args;
    type Output = Value;
    type Error = DomainToolError;

    fn description(&self) -> String {
        "按当前 eig-rs 研究目录搜索可用门面；explicitFunction 仅接受用户原文中的完整 ds_ 函数标识，Newton/牛顿法等算法名称放入 query；返回方法差异与禁用诊断。调用前加载 study-planning skill。".into()
    }

    fn parameters(&self) -> Value {
        object_schema(
            json!({
                "query":{"type":"string","maxLength":256},
                "explicitFunction":{"type":["string","null"],"maxLength":256}
            }),
            &["query"],
        )
    }

    fn annotations(&self) -> ToolAnnotations {
        ToolAnnotations::default().with_read_only(true)
    }

    async fn call(&self, _: &mut ToolContext, args: Args) -> Result<Value, Self::Error> {
        self.services.require_skill("study-planning").await?;
        serde_json::to_value(
            self.services
                .study_configurations
                .search(&args.query, args.explicit_function.as_deref()),
        )
        .map_err(|error| DomainToolError::new("serialization_error", error.to_string()))
    }
}
