use std::collections::HashMap;

use serde::Serialize;

use super::ComparisonError;
use crate::application::datasets::DatasetColumn;
use crate::db::repositories::RunDataset;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CompatibleValue {
    pub column: String,
    pub dtype: String,
    pub unit: Option<String>,
}

impl CompatibleValue {
    #[cfg(test)]
    pub(super) fn new(column: &str, dtype: &str, unit: Option<&str>) -> Self {
        Self {
            column: column.into(),
            dtype: dtype.into(),
            unit: unit.map(str::to_owned),
        }
    }
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CompatibilityDimensions {
    pub baseline_rows: i64,
    pub candidate_rows: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct CompatibilityFacts {
    pub baseline_dataset_id: String,
    pub candidate_dataset_id: String,
    pub result_name: String,
    pub stable_keys: Vec<String>,
    pub values: Vec<CompatibleValue>,
    pub baseline_schema: Vec<DatasetColumn>,
    pub candidate_schema: Vec<DatasetColumn>,
    pub dimensions: CompatibilityDimensions,
}

#[derive(Debug)]
pub(super) struct CompatiblePair {
    pub baseline: RunDataset,
    pub candidate: RunDataset,
    pub facts: CompatibilityFacts,
}

pub(super) struct Compatibility;

impl Compatibility {
    pub(super) fn select(
        left: Vec<RunDataset>,
        right: Vec<RunDataset>,
    ) -> Result<CompatiblePair, ComparisonError> {
        let by_name = right
            .into_iter()
            .map(|dataset| (dataset.result_name.clone(), dataset))
            .collect::<HashMap<_, _>>();
        for baseline in left {
            let Some(candidate) = by_name.get(&baseline.result_name) else {
                continue;
            };
            let baseline_columns: Vec<DatasetColumn> =
                serde_json::from_str(&baseline.schema_json).unwrap_or_default();
            let candidate_columns: Vec<DatasetColumn> =
                serde_json::from_str(&candidate.schema_json).unwrap_or_default();
            let candidate_by_name = candidate_columns
                .iter()
                .map(|column| (column.name.as_str(), column))
                .collect::<HashMap<_, _>>();
            let mut keys = baseline_columns
                .iter()
                .filter(|column| {
                    let name = column.name.to_ascii_lowercase();
                    (name == "id"
                        || name == "key"
                        || name.ends_with("_id")
                        || name == "name"
                        || name == "bus"
                        || name == "kind"
                        || name == "time"
                        || name == "time_s")
                        && rss_identifier(&column.name)
                        && candidate_by_name
                            .get(column.name.as_str())
                            .is_some_and(|other| other.dtype == column.dtype)
                })
                .map(|column| column.name.clone())
                .collect::<Vec<_>>();
            if keys.is_empty() {
                continue;
            }
            keys.sort();
            let mut values = Vec::new();
            for column in &baseline_columns {
                if keys.contains(&column.name)
                    || !numeric(&column.dtype)
                    || !rss_identifier(&column.name)
                {
                    continue;
                }
                let Some(other) = candidate_by_name.get(column.name.as_str()) else {
                    continue;
                };
                if other.dtype != column.dtype {
                    continue;
                }
                if other.unit != column.unit {
                    return Err(ComparisonError::new(
                        "comparison_incompatible",
                        "同名数值列单位不兼容",
                    ));
                }
                values.push(CompatibleValue {
                    column: column.name.clone(),
                    dtype: column.dtype.clone(),
                    unit: column.unit.clone(),
                });
            }
            if values.is_empty() {
                continue;
            }
            return Ok(CompatiblePair {
                facts: CompatibilityFacts {
                    baseline_dataset_id: baseline.id.to_string(),
                    candidate_dataset_id: candidate.id.to_string(),
                    result_name: baseline.result_name.clone(),
                    stable_keys: keys,
                    values,
                    baseline_schema: baseline_columns,
                    candidate_schema: candidate_columns,
                    dimensions: CompatibilityDimensions {
                        baseline_rows: baseline.row_count,
                        candidate_rows: candidate.row_count,
                    },
                },
                baseline,
                candidate: candidate.clone(),
            });
        }
        Err(ComparisonError::new(
            "comparison_incompatible",
            "找不到具有稳定键、兼容数值列和单位的数据集",
        ))
    }
}

fn numeric(value: &str) -> bool {
    ["Int", "UInt", "Float", "Decimal"]
        .iter()
        .any(|prefix| value.starts_with(prefix))
        || value == "int64"
        || value == "float64"
}

fn rss_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    chars
        .next()
        .is_some_and(|character| character.is_ascii_alphabetic() || character == '_')
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::db::repositories::RunDataset;

    fn dataset(id: i32, unit: Option<&str>, include_value: bool) -> RunDataset {
        let mut columns = vec![json!({
            "name":"bus","dtype":"Utf8","nullable":false,"unit":null
        })];
        if include_value {
            columns.push(json!({
                "name":"load","dtype":"Float64","nullable":false,"unit":unit
            }));
            columns.push(json!({
                "name":"bad-name","dtype":"Float64","nullable":false,"unit":unit
            }));
        }
        RunDataset {
            id,
            run_id: Some(id),
            analysis_run_id: None,
            artifact_id: Some(id),
            result_name: "result".into(),
            schema_json: serde_json::to_string(&columns).unwrap(),
            schema_hash: format!("schema-{id}"),
            row_count: 2,
            size_bytes: 10,
            created_at_ms: 1,
            deleted_at_ms: None,
            tombstone_json: None,
        }
    }

    #[test]
    fn selects_stable_keys_and_excludes_invalid_rss_identifiers() {
        let pair = Compatibility::select(
            vec![dataset(1, Some("MW"), true)],
            vec![dataset(2, Some("MW"), true)],
        )
        .unwrap();

        assert_eq!((pair.baseline.id, pair.candidate.id), (1, 2));
        assert_eq!(pair.facts.stable_keys, ["bus"]);
        assert_eq!(pair.facts.values[0].column, "load");
    }

    #[test]
    fn rejects_unit_mismatch_and_missing_numeric_values() {
        let unit_error = Compatibility::select(
            vec![dataset(1, Some("MW"), true)],
            vec![dataset(2, Some("kW"), true)],
        )
        .unwrap_err();
        assert_eq!(unit_error.code, "comparison_incompatible");

        let empty_error =
            Compatibility::select(vec![dataset(1, None, false)], vec![dataset(2, None, false)])
                .unwrap_err();
        assert_eq!(empty_error.code, "comparison_incompatible");
    }
}
