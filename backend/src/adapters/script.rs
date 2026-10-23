use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use super::{AdapterError, ErrorCategory, STATIC_MODEL_TABLES, validate_tree_file};

#[derive(Debug, Clone)]
pub struct ScriptValidationSpec {
    pub input_tree: PathBuf,
    pub script_path: PathBuf,
    pub timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StaticScriptValidation {
    pub valid: bool,
    pub required_aliases: Vec<String>,
    pub outputs: Vec<String>,
    pub diagnostics: serde_json::Value,
}

pub(crate) fn validate_script_source(
    spec: &ScriptValidationSpec,
) -> Result<StaticScriptValidation, AdapterError> {
    validate_tree_file(&spec.input_tree, &spec.script_path)?;
    let source =
        std::fs::read_to_string(spec.input_tree.join(&spec.script_path)).map_err(|error| {
            AdapterError::new("script_read_failed", ErrorCategory::Io, error.to_string())
        })?;
    let lower = source.to_ascii_lowercase();
    if lower.contains("import_")
        || lower.contains("read(")
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("file://")
    {
        return Ok(invalid("script_external_dependency_rejected"));
    }

    let mut outputs = BTreeSet::new();
    for statement in source.split([';', '\n']) {
        let Some((left, _right)) = statement.split_once('=') else {
            continue;
        };
        let name = left.trim();
        if valid_identifier(name) && !outputs.insert(name.to_string()) {
            return Ok(invalid("script_output_duplicate"));
        }
    }
    if outputs.is_empty() {
        return Ok(invalid("script_output_required"));
    }

    let mut required_aliases = STATIC_MODEL_TABLES
        .into_iter()
        .filter(|alias| contains_identifier(&lower, alias))
        .map(str::to_string)
        .collect::<Vec<_>>();
    required_aliases.retain(|alias| !outputs.contains(alias));
    required_aliases.sort();
    Ok(StaticScriptValidation {
        valid: true,
        required_aliases,
        outputs: outputs.into_iter().collect(),
        diagnostics: serde_json::json!({"code":"ok","validator":"adapter-v3"}),
    })
}

fn invalid(code: &str) -> StaticScriptValidation {
    StaticScriptValidation {
        valid: false,
        required_aliases: Vec::new(),
        outputs: Vec::new(),
        diagnostics: serde_json::json!({"code":code}),
    }
}

fn valid_identifier(value: &str) -> bool {
    let mut characters = value.chars();
    characters
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn contains_identifier(source: &str, needle: &str) -> bool {
    source.match_indices(needle).any(|(index, _)| {
        let before = source[..index].chars().next_back();
        let after = source[index + needle.len()..].chars().next();
        !before.is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
            && !after.is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}
