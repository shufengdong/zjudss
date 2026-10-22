use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{ExtractedTree, IngestionFormatError, RssInspector, safe_relative};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleFile {
    pub path: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleStudy {
    pub kind: Option<String>,
    pub solver: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BundleManifest {
    pub entry: String,
    pub files: Vec<BundleFile>,
    #[serde(default)]
    pub required_features: Vec<String>,
    #[serde(default)]
    pub study: Option<BundleStudy>,
}

#[derive(Debug, Clone)]
pub struct ValidatedBundle {
    pub root_prefix: String,
    pub manifest_path: String,
    pub manifest: BundleManifest,
    pub canonical_manifest: Value,
    pub diagnostics: Vec<String>,
}

pub struct BundleValidator;

impl BundleValidator {
    pub fn validate(tree: &ExtractedTree) -> Result<ValidatedBundle, IngestionFormatError> {
        let manifests: Vec<_> = tree
            .files
            .keys()
            .filter(|path| path.as_str() == "manifest.json" || path.ends_with("/manifest.json"))
            .cloned()
            .collect();
        if manifests.len() != 1 {
            return Err(IngestionFormatError::new(
                "bundle_manifest_count_invalid",
                manifests.len().to_string(),
            ));
        }
        let manifest_path = manifests[0].clone();
        let root_prefix = manifest_path
            .strip_suffix("manifest.json")
            .unwrap_or_default()
            .trim_end_matches('/')
            .to_string();
        if !root_prefix.is_empty() && !root_prefix.ends_with(".rssbundle") {
            return Err(IngestionFormatError::new(
                "bundle_root_invalid",
                root_prefix,
            ));
        }
        let bytes = std::fs::read(&tree.files[&manifest_path].path)
            .map_err(|error| IngestionFormatError::io("bundle_manifest_read_failed", error))?;
        let manifest: BundleManifest = serde_json::from_slice(&bytes).map_err(|error| {
            IngestionFormatError::new("bundle_manifest_invalid", error.to_string())
        })?;
        let entry = safe_relative(&manifest.entry)?;
        let mut seen = BTreeSet::new();
        let mut diagnostics = Vec::new();
        let mut canonical_files = Vec::new();
        for declared in &manifest.files {
            let relative = safe_relative(&declared.path)?;
            if !seen.insert(relative.clone()) {
                return Err(IngestionFormatError::new("bundle_duplicate_path", relative));
            }
            let tree_path = if root_prefix.is_empty() {
                relative.clone()
            } else {
                format!("{root_prefix}/{relative}")
            };
            match tree.files.get(&tree_path) {
                Some(file) if file.sha256 == declared.sha256 => canonical_files.push(BundleFile {
                    path: relative,
                    sha256: file.sha256.clone(),
                }),
                Some(_) => diagnostics.push(format!("hash mismatch: {}", declared.path)),
                None => diagnostics.push(format!("missing file: {}", declared.path)),
            }
        }
        if !seen.contains(&entry) {
            diagnostics.push("entry is not present in files".into());
        }
        canonical_files.sort_by(|left, right| left.path.cmp(&right.path));
        let entry_path = if root_prefix.is_empty() {
            entry.clone()
        } else {
            format!("{root_prefix}/{entry}")
        };
        if diagnostics.is_empty() {
            let report = RssInspector::inspect(&entry_path, tree)?;
            diagnostics.extend(report.diagnostics);
        }
        let canonical_manifest = serde_json::json!({
            "entry": entry,
            "files": canonical_files,
            "required_features": manifest.required_features,
            "study": manifest.study,
        });
        Ok(ValidatedBundle {
            root_prefix,
            manifest_path,
            manifest,
            canonical_manifest,
            diagnostics,
        })
    }
}
