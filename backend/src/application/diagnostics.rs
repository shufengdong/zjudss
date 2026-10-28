use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::adapters::AdapterError;

const MAX_SENT_BYTES: usize = 64 * 1024;
const TRUNCATION_MARKER: &str = "\n…[诊断已截断]…\n";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SanitizedDiagnostic {
    pub text: String,
    pub full_hash: String,
    pub retained_hash: String,
    pub sent_hash: String,
    pub total_bytes: u64,
    pub sent_bytes: u64,
    pub truncated: bool,
}

pub fn sanitize_adapter_stderr(error: &AdapterError) -> SanitizedDiagnostic {
    let stream = error
        .diagnostics
        .as_deref()
        .and_then(|value| value.get("stderr"));
    let retained = raw_adapter_stderr(error);
    let full_hash = stream
        .and_then(|value| value.get("fullSha256"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| hash(&retained));
    let retained_hash = stream
        .and_then(|value| value.get("retainedSha256"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| hash(&retained));
    let total_bytes = stream
        .and_then(|value| value.get("totalBytes"))
        .and_then(serde_json::Value::as_u64)
        .unwrap_or(retained.len() as u64);
    let sanitized = sanitize_text(&String::from_utf8_lossy(&retained));
    let (text, local_truncated) = truncate_utf8(&sanitized, MAX_SENT_BYTES);
    let truncated = local_truncated
        || stream
            .and_then(|value| value.get("truncated"))
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
    SanitizedDiagnostic {
        sent_hash: hash(text.as_bytes()),
        sent_bytes: text.len() as u64,
        text,
        full_hash,
        retained_hash,
        total_bytes,
        truncated,
    }
}

pub fn raw_adapter_stderr(error: &AdapterError) -> Vec<u8> {
    error
        .diagnostics
        .as_deref()
        .and_then(|value| value.get("stderr"))
        .and_then(|value| value.get("retained"))
        .and_then(serde_json::Value::as_array)
        .map(|bytes| {
            bytes
                .iter()
                .filter_map(serde_json::Value::as_u64)
                .filter_map(|value| u8::try_from(value).ok())
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| error.message.as_bytes().to_vec())
}

pub fn sanitize_text(value: &str) -> String {
    let cleaned = value
        .chars()
        .map(|value| {
            if value == '\n' || value == '\t' || !value.is_control() {
                value
            } else {
                '�'
            }
        })
        .collect::<String>();
    let mut output = Vec::new();
    let mut redact_bearer_value = false;
    let mut secret_value_pending = false;
    let mut secret_separator_pending = false;
    for token in cleaned.split_inclusive(char::is_whitespace) {
        let trimmed = token.trim_end_matches(char::is_whitespace);
        let suffix = &token[trimmed.len()..];
        let lower = trimmed.to_ascii_lowercase();
        if secret_value_pending
            && lower.trim_matches(|c: char| !c.is_ascii_alphanumeric()) == "bearer"
        {
            output.push(token.to_string());
        } else if redact_bearer_value || secret_value_pending {
            output.push(format!("[敏感信息已隐藏]{suffix}"));
            redact_bearer_value = false;
            secret_value_pending = false;
        } else if secret_separator_pending {
            if trimmed.chars().all(|value| "=:\"'`".contains(value)) {
                output.push(token.to_string());
                secret_value_pending = true;
            } else {
                output.push(format!("[敏感信息已隐藏]{suffix}"));
            }
            secret_separator_pending = false;
        } else if lower.trim_matches(|c: char| !c.is_ascii_alphanumeric()) == "bearer" {
            output.push(token.to_string());
            redact_bearer_value = true;
        } else if let Some(separator) = secret_assignment_separator(&lower) {
            output.push(format!("{}[敏感信息已隐藏]{suffix}", &trimmed[..separator]));
            if trimmed[separator..]
                .trim_matches(|value: char| "\"'`{}[](),;".contains(value))
                .is_empty()
            {
                secret_value_pending = true;
            }
        } else if is_secret_key(&lower) {
            output.push(token.to_string());
            secret_separator_pending = true;
        } else {
            output.push(redact_absolute_paths(trimmed));
            output.push(suffix.to_string());
        }
    }
    output.concat()
}

fn is_secret_key(lower: &str) -> bool {
    matches!(
        lower.trim_matches(|value: char| !value.is_ascii_alphanumeric() && value != '_'),
        "api_key" | "apikey" | "token" | "secret" | "authorization" | "password"
    )
}

fn secret_assignment_separator(lower: &str) -> Option<usize> {
    const NAMES: [&str; 6] = [
        "api_key",
        "apikey",
        "token",
        "secret",
        "authorization",
        "password",
    ];
    NAMES.iter().find_map(|name| {
        let name_at = lower.find(name)?;
        let tail = &lower[name_at + name.len()..];
        let relative = tail.find(['=', ':'])?;
        Some(name_at + name.len() + relative + 1)
    })
}

fn redact_absolute_paths(token: &str) -> String {
    let Some(start) = token.char_indices().find_map(|(index, value)| {
        (value == '/'
            && (index == 0
                || token[..index]
                    .chars()
                    .next_back()
                    .is_some_and(|previous| "\"'`=:(,[{".contains(previous))))
        .then_some(index)
    }) else {
        return token.to_string();
    };
    let trailing = token[start..]
        .char_indices()
        .rev()
        .take_while(|(_, value)| "\"'`),;]}".contains(*value))
        .last()
        .map(|(index, _)| start + index)
        .unwrap_or(token.len());
    format!(
        "{}[服务器路径已隐藏]{}",
        &token[..start],
        &token[trailing..]
    )
}

fn truncate_utf8(value: &str, limit: usize) -> (String, bool) {
    if value.len() <= limit {
        return (value.to_string(), false);
    }
    let available = limit.saturating_sub(TRUNCATION_MARKER.len());
    let mut head = available / 2;
    while !value.is_char_boundary(head) {
        head -= 1;
    }
    let mut tail = value.len().saturating_sub(available - head);
    while !value.is_char_boundary(tail) {
        tail += 1;
    }
    (
        format!("{}{}{}", &value[..head], TRUNCATION_MARKER, &value[tail..]),
        true,
    )
}

fn hash(value: &[u8]) -> String {
    hex::encode(Sha256::digest(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapters::{AdapterError, CapturedStream, ErrorCategory};

    #[test]
    fn redacts_paths_secrets_and_truncates_on_utf8_boundaries() {
        let source = format!(
            "/srv/private/run.rss token=hidden {} tail",
            "错".repeat(30_000)
        );
        let bytes = source.into_bytes();
        let hash = hash(&bytes);
        let stream = CapturedStream {
            retained: bytes.clone(),
            full_sha256: hash.clone(),
            retained_sha256: hash,
            total_bytes: bytes.len() as u64,
            truncated: false,
        };
        let error = AdapterError::new("engine_failed", ErrorCategory::Solver, "failed")
            .with_diagnostics(serde_json::json!({"stderr":stream}));
        let sanitized = sanitize_adapter_stderr(&error);
        assert!(sanitized.text.contains("[服务器路径已隐藏]"));
        assert!(sanitized.text.contains("token=[敏感信息已隐藏]"));
        assert!(!sanitized.text.contains("/srv/private"));
        assert!(!sanitized.text.contains("hidden"));
        assert!(sanitized.sent_bytes <= MAX_SENT_BYTES as u64);
        assert!(sanitized.truncated);
    }

    #[test]
    fn redacts_env_json_bearer_and_punctuated_paths() {
        let sanitized = sanitize_text(
            "DEEPSEEK_API_KEY=abc Bearer xyz Authorization: Bearer auth-secret {\"token\":\"json-secret\"} {\"token\": \"spaced-json\"} TOKEN = spaced-env at (/srv/a.rss),",
        );
        for secret in [
            "abc",
            "xyz",
            "auth-secret",
            "json-secret",
            "spaced-json",
            "spaced-env",
            "/srv/a.rss",
        ] {
            assert!(!sanitized.contains(secret), "leaked {secret}: {sanitized}");
        }
        assert!(sanitized.contains("[敏感信息已隐藏]"));
        assert!(sanitized.contains("[服务器路径已隐藏]"));
    }
}
