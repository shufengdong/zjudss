use serde_json::Value;

use crate::adapters::AdapterError;
use crate::agent::FailureAnalysisRequest;

use super::diagnostics::{
    failure_analysis_diagnostic, sanitize_adapter_stderr, sanitize_adapter_stdout,
};
use super::{LockedStudyContract, WorkflowInputSnapshot};

pub(crate) fn failure_analysis_request(
    current_script: String,
    adapter_error: &AdapterError,
    locked_contract: Option<Value>,
    inputs: Option<Value>,
) -> FailureAnalysisRequest {
    let stderr = sanitize_adapter_stderr(adapter_error);
    FailureAnalysisRequest {
        current_script,
        sanitized_stdout: sanitize_adapter_stdout(adapter_error).map(|stdout| stdout.text),
        sanitized_stderr: stderr.text,
        diagnostic: Some(failure_analysis_diagnostic(adapter_error)),
        locked_contract: locked_contract.map(|value| {
            serde_json::from_value::<LockedStudyContract>(value)
                .expect("controlled locked contract serializes")
        }),
        inputs: inputs.map(|value| {
            serde_json::from_value::<WorkflowInputSnapshot>(value)
                .expect("controlled workflow inputs serialize")
        }),
    }
}

#[cfg(test)]
mod tests {
    use crate::adapters::{AdapterError, ErrorCategory};

    use super::failure_analysis_request;

    #[test]
    fn request_uses_bounded_redacted_diagnostics() {
        let error = AdapterError::new(
            "engine_failed",
            ErrorCategory::Solver,
            format!("/private/secret/model {}", "错误".repeat(40_000)),
        );
        let request = failure_analysis_request("solve();".into(), &error, None, None);
        assert!(request.sanitized_stderr.len() <= 64 * 1024 + 64);
        assert!(!request.sanitized_stderr.contains("/private/secret/model"));
    }
}
