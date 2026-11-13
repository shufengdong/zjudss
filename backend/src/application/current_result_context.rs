use serde::Serialize;

use crate::application::dataframe_reader::DataFrameColumn;
use crate::application::runs::{RunError, RunService};
use crate::auth::CurrentUser;
use crate::db::repositories::{RepoError, RunDataset};

pub(crate) const CURRENT_RESULT_RUN_LIMIT: i64 = 20;
const CURRENT_RESULT_PROMPT_MAX_BYTES: usize = 65_536;
const CURRENT_RESULT_MAX_COLUMNS: usize = 32;
const CURRENT_RESULT_MAX_TEXT_CHARS: usize = 256;
const CURRENT_RESULT_MAX_DATASETS: usize = 100;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CurrentResultContext {
    pub(crate) run_id: String,
    pub(crate) created_at_ms: i64,
    pub(crate) datasets: Vec<CurrentResultDatasetContext>,
    pub(crate) datasets_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CurrentResultDatasetContext {
    pub(crate) dataset_id: String,
    pub(crate) name: String,
    pub(crate) name_truncated: bool,
    pub(crate) row_count: i64,
    pub(crate) size_bytes: i64,
    pub(crate) schema_hash: String,
    pub(crate) schema_hash_truncated: bool,
    pub(crate) columns: Vec<CurrentResultColumnContext>,
    pub(crate) columns_truncated: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CurrentResultColumnContext {
    pub(crate) name: String,
    pub(crate) dtype: String,
    pub(crate) nullable: bool,
    pub(crate) unit: Option<String>,
    pub(crate) truncated: bool,
}

impl CurrentResultContext {
    pub(crate) async fn resolve(
        user: &CurrentUser,
        session_id: i32,
        runs: &RunService,
    ) -> Result<Option<Self>, RunError> {
        for run in runs
            .list(user, session_id, CURRENT_RESULT_RUN_LIMIT)
            .await?
        {
            if run.status != "succeeded" {
                continue;
            }
            let datasets = runs.datasets(user, run.id).await?;
            if datasets.is_empty() {
                continue;
            }
            return Self::from_datasets(run.id, run.created_at_ms, datasets).map(Some);
        }
        Ok(None)
    }

    pub(crate) fn from_datasets(
        run_id: i32,
        created_at_ms: i64,
        datasets: Vec<RunDataset>,
    ) -> Result<Self, RunError> {
        let dataset_count = datasets.len();
        let mut context = Self {
            run_id: run_id.to_string(),
            created_at_ms,
            datasets: Vec::new(),
            datasets_truncated: dataset_count > CURRENT_RESULT_MAX_DATASETS,
        };
        let mut pending_columns = Vec::<Vec<DataFrameColumn>>::new();

        for dataset in datasets.into_iter().take(CURRENT_RESULT_MAX_DATASETS) {
            let columns = serde_json::from_str::<Vec<DataFrameColumn>>(&dataset.schema_json)
                .map_err(|error| {
                    RunError::Repository(RepoError::Storage(format!(
                        "invalid result schema for dataset {}: {error}",
                        dataset.id
                    )))
                })?;
            let (name, name_truncated) = bounded_text(&dataset.result_name);
            let (schema_hash, schema_hash_truncated) = bounded_text(&dataset.schema_hash);
            context.datasets.push(CurrentResultDatasetContext {
                dataset_id: dataset.id.to_string(),
                name,
                name_truncated,
                row_count: dataset.row_count,
                size_bytes: dataset.size_bytes,
                schema_hash,
                schema_hash_truncated,
                columns: Vec::new(),
                columns_truncated: !columns.is_empty(),
            });
            if context.encoded_len() > CURRENT_RESULT_PROMPT_MAX_BYTES {
                context.datasets.pop();
                context.datasets_truncated = true;
                break;
            }
            pending_columns.push(columns);
        }
        if context.datasets.len() < dataset_count {
            context.datasets_truncated = true;
        }

        for (index, columns) in pending_columns.into_iter().enumerate() {
            let capped_count = columns.len().min(CURRENT_RESULT_MAX_COLUMNS);
            for column in columns.iter().take(capped_count) {
                context.datasets[index].columns.push(bounded_column(column));
                context.datasets[index].columns_truncated =
                    context.datasets[index].columns.len() < columns.len();
                if context.encoded_len() > CURRENT_RESULT_PROMPT_MAX_BYTES {
                    context.datasets[index].columns.pop();
                    context.datasets[index].columns_truncated = true;
                    break;
                }
            }
            context.datasets[index].columns_truncated =
                context.datasets[index].columns.len() < columns.len();
        }

        debug_assert!(context.encoded_len() <= CURRENT_RESULT_PROMPT_MAX_BYTES);
        Ok(context)
    }

    pub(crate) fn prompt_json(&self) -> String {
        serde_json::to_string(self).expect("current result context is serializable")
    }

    fn encoded_len(&self) -> usize {
        serde_json::to_vec(self)
            .expect("current result context is serializable")
            .len()
    }
}

fn bounded_column(column: &DataFrameColumn) -> CurrentResultColumnContext {
    let (name, name_truncated) = bounded_text(&column.name);
    let (dtype, dtype_truncated) = bounded_text(&column.dtype);
    let (unit, unit_truncated) = match column.unit.as_deref() {
        Some(unit) => {
            let (unit, truncated) = bounded_text(unit);
            (Some(unit), truncated)
        }
        None => (None, false),
    };
    CurrentResultColumnContext {
        name,
        dtype,
        nullable: column.nullable,
        unit,
        truncated: name_truncated || dtype_truncated || unit_truncated,
    }
}

fn bounded_text(value: &str) -> (String, bool) {
    let mut chars = value.chars();
    let bounded = chars
        .by_ref()
        .take(CURRENT_RESULT_MAX_TEXT_CHARS)
        .collect::<String>();
    (bounded, chars.next().is_some())
}

#[cfg(test)]
mod tests {
    use serde_json::Value;

    use super::*;

    fn dataset(id: i32, name: String, columns: Value) -> RunDataset {
        RunDataset {
            id,
            run_id: Some(7),
            analysis_run_id: None,
            artifact_id: Some(id + 100),
            result_name: name,
            schema_json: columns.to_string(),
            schema_hash: format!("schema-{id}"),
            row_count: 10 + i64::from(id),
            size_bytes: 100 + i64::from(id),
            created_at_ms: 1,
            deleted_at_ms: None,
            tombstone_json: None,
        }
    }

    #[test]
    fn prompt_context_accounts_for_final_truncation_flag_at_exact_byte_boundary() {
        // All columns fit at 65,536 bytes while columnsTruncated is true, but
        // completing the schema changes it to false, which costs one more byte.
        let mut expected = CurrentResultContext {
            run_id: "7".into(),
            created_at_ms: 1,
            datasets: vec![CurrentResultDatasetContext {
                dataset_id: "1".into(),
                name: "voltage".into(),
                name_truncated: false,
                row_count: 11,
                size_bytes: 101,
                schema_hash: "schema-1".into(),
                schema_hash_truncated: false,
                columns: (0..28)
                    .map(|_| CurrentResultColumnContext {
                        name: String::new(),
                        dtype: String::new(),
                        nullable: false,
                        unit: Some(String::new()),
                        truncated: false,
                    })
                    .collect(),
                columns_truncated: false,
            }],
            datasets_truncated: false,
        };
        let mut remaining = CURRENT_RESULT_PROMPT_MAX_BYTES + 1 - expected.encoded_len();
        for column in &mut expected.datasets[0].columns {
            for field in [
                &mut column.name,
                &mut column.dtype,
                column.unit.as_mut().unwrap(),
            ] {
                let bytes = remaining.min(1_500);
                *field = "\0".repeat(bytes / 6) + &"x".repeat(bytes % 6);
                remaining -= bytes;
            }
        }
        assert_eq!(remaining, 0);
        assert_eq!(expected.encoded_len(), CURRENT_RESULT_PROMPT_MAX_BYTES + 1);
        let columns = serde_json::to_value(&expected.datasets[0].columns).unwrap();
        let context =
            CurrentResultContext::from_datasets(7, 1, vec![dataset(1, "voltage".into(), columns)])
                .unwrap();
        assert!(context.prompt_json().len() <= CURRENT_RESULT_PROMPT_MAX_BYTES);
        assert!(context.datasets[0].columns_truncated);
        assert_eq!(context.datasets[0].columns.len(), 27);
        assert_eq!(
            context.datasets[0].columns[0].name,
            expected.datasets[0].columns[0].name
        );
    }

    #[test]
    fn prompt_context_caps_columns_text_and_total_bytes() {
        let long = "电".repeat(300);
        let columns = Value::Array(
            (0..80)
                .map(|index| {
                    serde_json::json!({
                        "name":format!("{long}-{index}"),
                        "dtype":long,
                        "nullable":true,
                        "unit":long,
                    })
                })
                .collect(),
        );
        let datasets = (0..150)
            .map(|id| dataset(id, format!("{long}-{id}"), columns.clone()))
            .collect();

        let context = CurrentResultContext::from_datasets(7, 1234, datasets).unwrap();
        let encoded = context.prompt_json();
        let value: Value = serde_json::from_str(&encoded).unwrap();

        assert!(encoded.len() <= 65_536, "{}", encoded.len());
        assert_eq!(value["runId"], "7");
        assert!(!value["datasets"].as_array().unwrap().is_empty());
        assert!(value["datasets"].as_array().unwrap().len() <= 100);
        assert_eq!(value["datasetsTruncated"], true);
        assert_eq!(
            value["datasets"][0]["name"]
                .as_str()
                .unwrap()
                .chars()
                .count(),
            256
        );
        assert_eq!(value["datasets"][0]["nameTruncated"], true);
        assert!(value["datasets"][0]["columns"].as_array().unwrap().len() <= 32);
    }

    #[test]
    fn prompt_context_marks_dataset_and_column_truncation() {
        let columns = Value::Array(
            (0..33)
                .map(|index| {
                    serde_json::json!({
                        "name":format!("column-{index}"),
                        "dtype":"Float64",
                        "nullable":false,
                        "unit":"pu",
                    })
                })
                .collect(),
        );

        let context = CurrentResultContext::from_datasets(
            9,
            5678,
            vec![dataset(3, "bus_voltage".into(), columns)],
        )
        .unwrap();
        let value: Value = serde_json::from_str(&context.prompt_json()).unwrap();

        assert_eq!(value["datasetsTruncated"], false);
        assert_eq!(value["datasets"][0]["datasetId"], "3");
        assert_eq!(
            value["datasets"][0]["columns"].as_array().unwrap().len(),
            32
        );
        assert_eq!(value["datasets"][0]["columnsTruncated"], true);
        assert_eq!(value["datasets"][0]["columns"][0]["name"], "column-0");
        assert_eq!(value["datasets"][0]["columns"][0]["truncated"], false);
    }
}
