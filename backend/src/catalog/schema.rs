use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use crate::engine_resources::EngineResourceSnapshot;

#[derive(Debug, Clone)]
pub struct FrameSchema {
    pub kind: String,
    pub required: BTreeMap<String, String>,
    pub optional: BTreeMap<String, String>,
    pub stable_keys: BTreeSet<String>,
}

#[derive(Debug, Clone)]
pub struct SchemaCatalog {
    pub contract_version: String,
    pub schema_version: u32,
    pub frames: Vec<FrameSchema>,
}

#[derive(Debug)]
pub struct CatalogError {
    code: &'static str,
    message: String,
}

impl CatalogError {
    fn new(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub fn code(&self) -> &'static str {
        self.code
    }
}

impl std::fmt::Display for CatalogError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}
impl std::error::Error for CatalogError {}

impl SchemaCatalog {
    pub fn from_snapshot(
        snapshot: &EngineResourceSnapshot,
        contract_version: &str,
    ) -> Result<Self, CatalogError> {
        if snapshot.contract_version != contract_version {
            return Err(CatalogError::new(
                "engine_contract_mismatch",
                "请求合同与资源快照不一致",
            ));
        }
        Self::from_value(contract_version, &snapshot.catalog_json)
    }

    pub fn from_value(contract_version: &str, root: &Value) -> Result<Self, CatalogError> {
        let schema_version = root
            .get("schema_version")
            .and_then(Value::as_u64)
            .ok_or_else(|| CatalogError::new("catalog_invalid", "缺少 schema_version"))?
            as u32;
        let mut by_kind = BTreeMap::new();
        for section in ["frames", "analysis_inputs"] {
            if let Some(entries) = root.get(section).and_then(Value::as_object) {
                for (kind, definition) in entries {
                    if by_kind.contains_key(kind) {
                        continue;
                    }
                    if let Some(frame) = parse_frame(kind, definition) {
                        by_kind.insert(kind.clone(), frame);
                    }
                }
            }
        }
        if by_kind.is_empty() {
            return Err(CatalogError::new(
                "catalog_invalid",
                "catalog 没有可推断输入 schema",
            ));
        }
        Ok(Self {
            contract_version: contract_version.to_string(),
            schema_version,
            frames: by_kind.into_values().collect(),
        })
    }
}

fn parse_frame(kind: &str, value: &Value) -> Option<FrameSchema> {
    let required = [
        "required",
        "required_simple",
        "required_for_imported_network",
        "required_for_replay",
    ]
    .iter()
    .find_map(|name| value.get(*name).and_then(Value::as_object))
    .map(parse_columns)
    .unwrap_or_default();
    if required.is_empty() {
        return None;
    }
    let mut optional = value
        .get("optional")
        .and_then(Value::as_object)
        .map(parse_columns)
        .unwrap_or_default();
    if let Some(names) = value.get("optional_columns").and_then(Value::as_array) {
        for name in names.iter().filter_map(Value::as_str) {
            optional
                .entry(normalize_name(name))
                .or_insert_with(|| "unknown".to_string());
        }
    }
    let mut stable_keys = BTreeSet::new();
    if let Some(keys) = value.get("stable_keys").and_then(Value::as_array) {
        stable_keys.extend(keys.iter().filter_map(Value::as_str).map(normalize_name));
    }
    if stable_keys.is_empty() {
        for candidate in ["id", "name"] {
            if required.contains_key(candidate) {
                stable_keys.insert(candidate.to_string());
            }
        }
    }
    Some(FrameSchema {
        kind: kind.to_string(),
        required,
        optional,
        stable_keys,
    })
}

fn parse_columns(columns: &serde_json::Map<String, Value>) -> BTreeMap<String, String> {
    columns
        .iter()
        .map(|(name, dtype)| {
            (
                normalize_name(name),
                dtype.as_str().unwrap_or("unknown").to_string(),
            )
        })
        .collect()
}

pub(crate) fn normalize_name(name: &str) -> String {
    name.trim_start_matches('\u{feff}').trim().to_lowercase()
}
