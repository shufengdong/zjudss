use std::collections::BTreeMap;

use rig::completion::ToolAnnotations;
use rig::tool::{Tool, ToolContext, ToolErrorKind, ToolExecutionError};
use sha2::{Digest, Sha256};

use crate::application::study_planning::ReadyStudyPlan;

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
        let feedback = format!("{}: {}", error.code, error.message);
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

#[derive(Debug, Default)]
pub struct ReadyStudyPlanRegistry {
    plans: BTreeMap<String, ReadyStudyPlan>,
}

impl ReadyStudyPlanRegistry {
    pub fn insert(&mut self, request_message_id: i32, plan: ReadyStudyPlan) -> String {
        let proof = serde_json::json!({
            "requestMessageId": request_message_id,
            "plan": &plan,
        });
        let digest = Sha256::digest(crate::application::canonical_json(&proof).as_bytes());
        let id = format!("study-plan:{}", hex::encode(digest));
        self.plans.insert(id.clone(), plan);
        id
    }

    pub fn get(&self, id: &str) -> Option<ReadyStudyPlan> {
        self.plans.get(id).cloned()
    }
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
        Self::new(value.code, value.to_string())
    }
}
impl From<crate::application::ComparisonError> for DomainToolError {
    fn from(value: crate::application::ComparisonError) -> Self {
        Self::new(value.code, value.to_string())
    }
}

impl From<crate::application::study_planning::StudyPlanningError> for DomainToolError {
    fn from(value: crate::application::study_planning::StudyPlanningError) -> Self {
        Self::new(value.code(), value.to_string())
    }
}

pub fn object_schema(properties: serde_json::Value, required: &[&str]) -> serde_json::Value {
    serde_json::json!({"type":"object","properties":properties,"required":required,"additionalProperties":false})
}
