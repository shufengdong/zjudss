//! 薄工具包装器共享的领域错误适配、严格对象 schema 与 Rig dispatch 边界。

use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext, ToolErrorKind, ToolExecutionError};

const MAX_MODEL_FEEDBACK_BYTES: usize = 64 * 1024;
const FEEDBACK_TRUNCATION_MARKER: &str = "…[工具反馈已截断]…";

#[derive(Debug)]
/// 向模型返回稳定 code 与可读反馈的领域工具错误。
pub struct DomainToolError {
    pub code: &'static str,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

impl DomainToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn with_details(mut self, details: serde_json::Value) -> Self {
        self.details = Some(details);
        self
    }
}
impl std::fmt::Display for DomainToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for DomainToolError {}

/// Normalizes deliberate domain failures at Rig's erased dispatch boundary while keeping direct
/// typed tool calls unchanged for ordinary unit tests.
pub struct DomainToolAdapter<T> {
    inner: T,
}

impl<T> DomainToolAdapter<T> {
    pub fn new(inner: T) -> Self {
        Self { inner }
    }
}

impl<T> Tool for DomainToolAdapter<T>
where
    T: Tool<Error = DomainToolError>,
{
    const NAME: &'static str = T::NAME;
    type Args = T::Args;
    type Output = T::Output;
    type Error = DomainToolError;

    fn description(&self) -> String {
        self.inner.description()
    }

    fn parameters(&self) -> serde_json::Value {
        self.inner.parameters()
    }

    fn annotations(&self) -> ToolAnnotations {
        self.inner.annotations()
    }

    fn map_error(&self, error: Self::Error) -> ToolExecutionError {
        let feedback = bounded_model_feedback(&error);
        ToolExecutionError::new(ToolErrorKind::Other, error.to_string())
            .with_model_feedback(feedback)
            .with_retryable(false)
            .with_code(error.code)
            .with_source(error)
    }

    async fn call(
        &self,
        context: &mut ToolContext,
        args: Self::Args,
    ) -> Result<Self::Output, Self::Error> {
        self.inner.call(context, args).await
    }
}

fn bounded_model_feedback(error: &DomainToolError) -> String {
    let prefix = format!("{}: {}", error.code, error.message);
    let Some(mut details) = error.details.clone() else {
        return truncate_utf8(prefix, MAX_MODEL_FEEDBACK_BYTES);
    };
    for _ in 0..4 {
        let serialized = serde_json::to_string(&details).expect("tool error details serialize");
        let feedback = format!("{prefix}\n{serialized}");
        if feedback.len() <= MAX_MODEL_FEEDBACK_BYTES {
            return feedback;
        }
        let excess = feedback.len() - MAX_MODEL_FEEDBACK_BYTES;
        let Some(text) = details
            .pointer_mut("/diagnostic/text")
            .and_then(|value| value.as_str())
            .map(str::to_string)
        else {
            break;
        };
        let target = text
            .len()
            .saturating_sub(excess + FEEDBACK_TRUNCATION_MARKER.len() + 32);
        let shortened = truncate_utf8(text, target);
        details["diagnostic"]["text"] =
            serde_json::Value::String(format!("{shortened}{FEEDBACK_TRUNCATION_MARKER}"));
        details["diagnostic"]["truncated"] = serde_json::Value::Bool(true);
    }
    let summary = serde_json::json!({
        "code": details.get("code"),
        "repairable": details.get("repairable"),
        "stage": details.get("stage"),
        "diagnostic": {"truncated": true},
    });
    truncate_utf8(
        format!(
            "{prefix}\n{}",
            serde_json::to_string(&summary).expect("tool error summary serializes")
        ),
        MAX_MODEL_FEEDBACK_BYTES,
    )
}

fn truncate_utf8(mut value: String, limit: usize) -> String {
    if value.len() <= limit {
        return value;
    }
    let mut end = limit;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

impl From<crate::application::VersionError> for DomainToolError {
    fn from(value: crate::application::VersionError) -> Self {
        Self::new("workspace_error", value.to_string())
    }
}
impl From<crate::application::ModelMutationError> for DomainToolError {
    fn from(value: crate::application::ModelMutationError) -> Self {
        Self::new(value.code(), value.to_string())
    }
}
impl From<crate::application::ScriptError> for DomainToolError {
    fn from(value: crate::application::ScriptError) -> Self {
        Self::new(value.code(), value.to_string())
    }
}
impl From<crate::application::AnalysisError> for DomainToolError {
    fn from(value: crate::application::AnalysisError) -> Self {
        let details = value.details.clone();
        let error = Self::new(
            value.code,
            crate::application::diagnostics::sanitize_text(&value.to_string()),
        );
        match details {
            Some(details) => error.with_details(details),
            None => error,
        }
    }
}
impl From<crate::application::ComparisonError> for DomainToolError {
    fn from(value: crate::application::ComparisonError) -> Self {
        Self::new(value.code, value.to_string())
    }
}

impl From<crate::application::StudyConfigurationError> for DomainToolError {
    fn from(value: crate::application::StudyConfigurationError) -> Self {
        Self::new(value.code(), value.to_string())
    }
}

/// 构造拒绝额外字段的严格对象参数 schema。
pub fn object_schema(properties: serde_json::Value, required: &[&str]) -> serde_json::Value {
    serde_json::json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
