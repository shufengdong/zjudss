use std::collections::{BTreeSet, VecDeque};
use std::path::{Component, Path};

use serde::Serialize;

use super::{ExtractedTree, IngestionFormatError, safe_relative};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DependencyReport {
    pub entry: String,
    pub dependencies: Vec<String>,
    pub missing: Vec<String>,
    pub diagnostics: Vec<String>,
}

pub struct RssInspector;

impl RssInspector {
    pub fn inspect(
        entry: &str,
        tree: &ExtractedTree,
    ) -> Result<DependencyReport, IngestionFormatError> {
        let entry = safe_relative(entry)?;
        if !tree.files.contains_key(&entry) {
            return Err(IngestionFormatError::new("rss_entry_missing", entry));
        }
        let mut queue = VecDeque::from([entry.clone()]);
        let mut visited = BTreeSet::new();
        let mut dependencies = BTreeSet::new();
        let mut missing = BTreeSet::new();
        while let Some(current) = queue.pop_front() {
            if !visited.insert(current.clone()) {
                continue;
            }
            let file = &tree.files[&current];
            let bytes = std::fs::read(&file.path)
                .map_err(|error| IngestionFormatError::io("rss_read_failed", error))?;
            let text = String::from_utf8(bytes).map_err(|error| {
                IngestionFormatError::new("rss_invalid_utf8", error.to_string())
            })?;
            for token in dependency_tokens(&text) {
                let candidate = resolve_reference(&current, &token)?;
                if tree.files.contains_key(&candidate) {
                    if dependencies.insert(candidate.clone()) {
                        queue.push_back(candidate);
                    }
                } else {
                    missing.insert(token);
                }
            }
        }
        let missing: Vec<_> = missing.into_iter().collect();
        Ok(DependencyReport {
            entry,
            dependencies: dependencies.into_iter().collect(),
            diagnostics: missing
                .iter()
                .map(|path| format!("missing dependency: {path}"))
                .collect(),
            missing,
        })
    }
}

fn dependency_tokens(text: &str) -> Vec<String> {
    text.split(['"', '\''])
        .enumerate()
        .filter_map(|(index, token)| {
            if index % 2 == 0 {
                return None;
            }
            let lower = token.to_ascii_lowercase();
            [".dss", ".csv", ".json", ".rss"]
                .iter()
                .any(|suffix| lower.ends_with(suffix))
                .then(|| token.to_string())
        })
        .collect()
}

fn resolve_reference(current: &str, reference: &str) -> Result<String, IngestionFormatError> {
    if reference.contains("://") {
        return Err(IngestionFormatError::new(
            "rss_external_dependency_forbidden",
            reference,
        ));
    }
    let reference = safe_relative(reference)?;
    if reference.starts_with("sparrowzz/") || reference.starts_with("psa/") {
        return Ok(reference);
    }
    let parent = Path::new(current).parent().unwrap_or_else(|| Path::new(""));
    let joined = parent.join(&reference);
    if joined
        .components()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Err(IngestionFormatError::new(
            "rss_dependency_escape",
            reference,
        ));
    }
    Ok(joined
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/"))
}
