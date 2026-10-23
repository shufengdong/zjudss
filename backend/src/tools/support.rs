#[derive(Debug)]
pub struct DomainToolError {
    pub code: &'static str,
    pub message: String,
}

impl DomainToolError {
    pub fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
impl std::fmt::Display for DomainToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for DomainToolError {}

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
        Self::new("script_error", value.to_string())
    }
}
impl From<crate::application::AnalysisError> for DomainToolError {
    fn from(value: crate::application::AnalysisError) -> Self {
        Self::new(value.code, value.to_string())
    }
}
impl From<crate::application::ComparisonError> for DomainToolError {
    fn from(value: crate::application::ComparisonError) -> Self {
        Self::new(value.code, value.to_string())
    }
}

pub fn object_schema(properties: serde_json::Value, required: &[&str]) -> serde_json::Value {
    serde_json::json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
