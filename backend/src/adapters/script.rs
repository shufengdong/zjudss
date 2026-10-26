use std::collections::BTreeSet;
use std::path::PathBuf;
use std::time::Duration;

use super::{AdapterError, ErrorCategory, STATIC_MODEL_TABLES, validate_tree_file};

pub const GOVERNED_STUDY_FUNCTIONS: [&str; 7] = [
    "ds_radial_pf",
    "ds_newton_raphson_pf",
    "ds_radial_qsts",
    "ds_newton_raphson_qsts",
    "ds_fault_study",
    "ds_harmonic_study",
    "ds_motor_dynamics",
];

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
    pub called_functions: Vec<String>,
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
    let callable_source = strip_comments_and_strings(&source).to_ascii_lowercase();
    let called_functions = GOVERNED_STUDY_FUNCTIONS
        .into_iter()
        .filter(|name| contains_call(&callable_source, name))
        .map(str::to_string)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    if lower.contains("import_")
        || lower.contains("read(")
        || lower.contains("http://")
        || lower.contains("https://")
        || lower.contains("file://")
    {
        return Ok(invalid(
            "script_external_dependency_rejected",
            called_functions,
        ));
    }
    if has_trailing_bare_identifier(&callable_source) {
        return Ok(invalid("script_bare_output_rejected", called_functions));
    }

    let mut outputs = BTreeSet::new();
    for statement in source.split([';', '\n']) {
        let Some((left, _right)) = statement.split_once('=') else {
            continue;
        };
        let Some(names) = output_names(left) else {
            continue;
        };
        for name in names {
            if !outputs.insert(name.to_string()) {
                return Ok(invalid("script_output_duplicate", called_functions));
            }
        }
    }
    if outputs.is_empty() {
        return Ok(invalid("script_output_required", called_functions));
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
        called_functions,
        diagnostics: serde_json::json!({"code":"ok","validator":"adapter-v3"}),
    })
}

fn has_trailing_bare_identifier(source: &str) -> bool {
    let source = source.trim();
    let source = source.strip_suffix(';').unwrap_or(source).trim_end();
    let statement = source
        .rsplit_once(';')
        .map_or(source, |(_, statement)| statement)
        .trim();
    valid_identifier(statement)
}

fn invalid(code: &str, called_functions: Vec<String>) -> StaticScriptValidation {
    StaticScriptValidation {
        valid: false,
        required_aliases: Vec::new(),
        outputs: Vec::new(),
        called_functions,
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

fn output_names(binding: &str) -> Option<Vec<&str>> {
    let binding = binding.trim();
    if valid_identifier(binding) {
        return Some(vec![binding]);
    }

    let slots = binding.strip_prefix('[')?.strip_suffix(']')?;
    let mut names = Vec::new();
    for slot in slots.split(',') {
        let name = slot.trim();
        if name == "_" {
            continue;
        }
        if !valid_identifier(name) {
            return None;
        }
        names.push(name);
    }
    Some(names)
}

fn contains_identifier(source: &str, needle: &str) -> bool {
    source.match_indices(needle).any(|(index, _)| {
        let before = source[..index].chars().next_back();
        let after = source[index + needle.len()..].chars().next();
        !before.is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
            && !after.is_some_and(|character| character.is_ascii_alphanumeric() || character == '_')
    })
}

fn contains_call(source: &str, name: &str) -> bool {
    source.match_indices(name).any(|(index, _)| {
        let before = source[..index].chars().next_back();
        let after_name = &source[index + name.len()..];
        let identifier_boundary =
            !before.is_some_and(|character| character.is_ascii_alphanumeric() || character == '_');
        identifier_boundary && after_name.trim_start().starts_with('(')
    })
}

fn strip_comments_and_strings(source: &str) -> String {
    #[derive(Clone, Copy)]
    enum State {
        Code,
        SingleQuoted,
        DoubleQuoted,
        LineComment,
    }

    let characters = source.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(source.len());
    let mut state = State::Code;
    let mut escaped = false;
    let mut index = 0;
    while index < characters.len() {
        let character = characters[index];
        match state {
            State::Code if character == '#' => {
                state = State::LineComment;
                output.push(' ');
            }
            State::Code if character == '/' && characters.get(index + 1).copied() == Some('/') => {
                state = State::LineComment;
                output.push_str("  ");
                index += 1;
            }
            State::Code if character == '\'' => {
                state = State::SingleQuoted;
                escaped = false;
                output.push(' ');
            }
            State::Code if character == '"' => {
                state = State::DoubleQuoted;
                escaped = false;
                output.push(' ');
            }
            State::Code => output.push(character),
            State::LineComment if character == '\n' => {
                state = State::Code;
                output.push('\n');
            }
            State::LineComment => output.push(' '),
            State::SingleQuoted | State::DoubleQuoted => {
                let closing = matches!(state, State::SingleQuoted) && character == '\''
                    || matches!(state, State::DoubleQuoted) && character == '"';
                if closing && !escaped {
                    state = State::Code;
                }
                escaped = character == '\\' && !escaped;
                if character != '\\' {
                    escaped = false;
                }
                output.push(if character == '\n' { '\n' } else { ' ' });
            }
        }
        index += 1;
    }
    output
}
