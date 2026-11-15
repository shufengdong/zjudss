use serde_json::{Value, json};

use crate::adapters::{AdapterError, ErrorCategory};
use crate::application::diagnostics::{sanitize_adapter_stderr, sanitize_text};
use crate::engine_resources::EngineResourceSnapshot;

use super::AnalysisError;

const MAX_FEEDBACK_BYTES: usize = 64 * 1024;

pub(super) fn adapter_failure(
    error: &AdapterError,
    snapshot: &EngineResourceSnapshot,
) -> AnalysisError {
    let sanitized = sanitize_adapter_stderr(error);
    let repairable = is_repairable_engine_diagnostic(error, &sanitized.text);
    let code = if repairable {
        "analysis_engine_failed"
    } else {
        infrastructure_code(&error.code)
    };
    AnalysisError::failure(
        code,
        if repairable {
            "分析脚本执行失败，可根据诊断修正后重试"
        } else {
            "分析执行基础设施失败"
        },
        repairable,
        "run",
        json!({
            "engineCode": error.code,
            "category": error.category.as_str(),
            "text": sanitized.text,
            "truncated": sanitized.truncated,
            "fullHash": sanitized.full_hash,
            "retainedHash": sanitized.retained_hash,
            "sentHash": sanitized.sent_hash,
            "totalBytes": sanitized.total_bytes,
            "sentBytes": sanitized.sent_bytes,
            "engineSnapshotId": snapshot.id,
        }),
    )
}

fn is_repairable_engine_diagnostic(error: &AdapterError, diagnostic: &str) -> bool {
    if error.category == ErrorCategory::Io || error.code.starts_with("result_") {
        return false;
    }
    if error.category == ErrorCategory::Model || error.code == "model_error" {
        return true;
    }
    let diagnostic = diagnostic.to_ascii_lowercase();
    [
        "parse",
        "syntax",
        "unexpected token",
        "unknown function",
        "unknown column",
        "columnnotfound",
        "column not found",
        "dtype",
        "type mismatch",
        "arity",
        "argument",
        "list",
        "join",
        "aggregate",
    ]
    .iter()
    .any(|needle| diagnostic.contains(needle))
}

pub(super) fn output_contract_failure(expected: &[String], actual: &[String]) -> AnalysisError {
    AnalysisError::failure(
        "analysis_result_contract_invalid",
        "实际输出与声明输出不一致，可修正脚本输出后重试",
        true,
        "collect",
        json!({"expectedOutputs": expected, "actualOutputs": actual}),
    )
}

pub(super) fn nonrepairable(
    code: &'static str,
    message: impl Into<String>,
    stage: &'static str,
    diagnostic: impl AsRef<str>,
) -> AnalysisError {
    AnalysisError::failure(
        code,
        message,
        false,
        stage,
        json!({"text": bounded_sanitized_text(diagnostic.as_ref())}),
    )
}

fn infrastructure_code(code: &str) -> &'static str {
    match code {
        "result_monitor_failed" => "result_monitor_failed",
        "result_file_limit" => "result_file_limit",
        "result_file_too_large" => "result_file_too_large",
        "result_total_too_large" => "result_total_too_large",
        "result_symlink_rejected" => "result_symlink_rejected",
        "result_path_escape" => "result_path_escape",
        _ => "analysis_engine_failed",
    }
}

fn bounded_sanitized_text(value: &str) -> String {
    let value = sanitize_text(value);
    if value.len() <= MAX_FEEDBACK_BYTES {
        return value;
    }
    let mut end = MAX_FEEDBACK_BYTES;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].to_string()
}

pub(super) fn stored_failure(error: &AnalysisError) -> Value {
    json!({
        "code": error.code,
        "message": sanitize_text(&error.message),
        "details": error.details,
    })
}
