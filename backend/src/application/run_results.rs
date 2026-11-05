use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};

use crate::auth::CurrentUser;
use crate::catalog::{DataTypeContract, SchemaFieldContract};
use crate::db::repositories::{
    ArtifactDraft, NewRunDataset, RepoError, RunArtifactOutputs, RunDataset, RunRepo,
};
use crate::engine_resources::EngineResourceSnapshot;
use crate::storage::{ArtifactStore, ParquetMetadata, StageLimits};

use super::execution_results::InspectedExecutionOutput;
use super::{ArtifactError, ArtifactService, FrozenResultContract, RunError, now_ms};

pub(crate) struct OpenResultPolicy;

impl OpenResultPolicy {
    pub fn validate(outputs: &[InspectedExecutionOutput]) -> Result<(), RunError> {
        if outputs.is_empty() {
            return Err(RunError::ResultContract(
                "no_result_dataset",
                "脚本没有生成结果数据集".into(),
            ));
        }
        let names = outputs
            .iter()
            .map(|output| output.name.as_str())
            .collect::<BTreeSet<_>>();
        if names.len() != outputs.len() || names.iter().any(|name| !valid_result_name(name)) {
            return Err(RunError::ResultContract(
                "result_contract_invalid",
                "结果名称不合法或重复".into(),
            ));
        }
        Ok(())
    }
}

pub(crate) struct FrozenResultPolicy<'a> {
    pub snapshot: &'a EngineResourceSnapshot,
    pub contract: &'a [FrozenResultContract],
}

impl FrozenResultPolicy<'_> {
    pub fn validate(&self, outputs: &[InspectedExecutionOutput]) -> Result<(), RunError> {
        OpenResultPolicy::validate(outputs)?;
        let actual = outputs
            .iter()
            .map(|output| output.name.as_str())
            .collect::<BTreeSet<_>>();
        let expected = self
            .contract
            .iter()
            .map(|result| result.name.as_str())
            .collect::<BTreeSet<_>>();
        if expected != actual || self.contract.len() != outputs.len() {
            return Err(RunError::ResultContract(
                "result_contract_invalid",
                "结果名称集合与冻结合同不一致".into(),
            ));
        }
        for expected in self.contract {
            let output = outputs
                .iter()
                .find(|output| output.name == expected.name)
                .expect("name set checked");
            let schema = self
                .snapshot
                .schemas
                .resolve(&expected.schema_ref)
                .map_err(|_| {
                    RunError::ResultContract(
                        "result_contract_invalid",
                        "冻结结果 schema 不可用".into(),
                    )
                })?;
            validate_frozen_schema(&output.metadata, &schema.fields)
                .map_err(|message| RunError::ResultContract("result_contract_invalid", message))?;
            validate_frozen_non_null_values(&output.path, &schema.fields)
                .map_err(|message| RunError::ResultContract("result_contract_invalid", message))?;
        }
        Ok(())
    }
}

fn validate_frozen_schema(
    metadata: &ParquetMetadata,
    expected: &[SchemaFieldContract],
) -> Result<(), String> {
    if metadata.columns.len() != expected.len() {
        return Err(format!(
            "结果列数不一致: expected {}, actual {}",
            expected.len(),
            metadata.columns.len()
        ));
    }
    for (actual, expected) in metadata.columns.iter().zip(expected) {
        let expected_dtype = canonical_arrow(&expected.dtype);
        let expected_unit =
            (expected.unit.0 != "not_applicable").then_some(expected.unit.0.as_str());
        let unit_mismatch = actual
            .unit
            .as_deref()
            .is_some_and(|actual| Some(actual) != expected_unit);
        if actual.name != expected.name
            || !dtype_matches(&actual.dtype, &expected.dtype)
            || unit_mismatch
        {
            return Err(format!(
                "结果列 {} 不符合合同: expected {}/{}/{:?}, actual {}/{}/{:?}",
                expected.name,
                expected_dtype,
                expected.nullable,
                expected_unit,
                actual.dtype,
                actual.nullable,
                actual.unit
            ));
        }
    }
    Ok(())
}

fn validate_frozen_non_null_values(
    path: &std::path::Path,
    expected: &[SchemaFieldContract],
) -> Result<(), String> {
    let columns = expected
        .iter()
        .filter(|field| !field.nullable)
        .map(|field| field.name.clone())
        .collect::<Vec<_>>();
    match crate::storage::ParquetInspector::first_column_with_nulls(path, &columns) {
        Ok(Some(name)) => Err(format!("结果列 {name} 含 null，不符合非空合同")),
        Ok(None) => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn canonical_arrow(dtype: &DataTypeContract) -> String {
    match dtype {
        DataTypeContract::Primitive { name } => match name.as_str() {
            "str" => "Utf8".into(),
            "bool" => "Boolean".into(),
            "i32" => "Int32".into(),
            "i64" => "Int64".into(),
            "u32" => "UInt32".into(),
            "u64" => "UInt64".into(),
            "f32" => "Float32".into(),
            "f64" => "Float64".into(),
            other => other.into(),
        },
        DataTypeContract::Semantic { name } => match name.as_str() {
            "u32_semantics" => "UInt32".into(),
            other => other.into(),
        },
        DataTypeContract::Constructor { name, element } => {
            format!("{}<{}>", title_case(name), canonical_arrow(element))
        }
    }
}

fn dtype_matches(actual: &str, expected: &DataTypeContract) -> bool {
    match expected {
        DataTypeContract::Primitive { name } if name == "str" => {
            matches!(actual, "Utf8" | "LargeUtf8" | "Utf8View")
        }
        DataTypeContract::Constructor { name, element }
            if matches!(name.as_str(), "list" | "matrix") =>
        {
            let element = canonical_arrow(element);
            ["List", "LargeList", "ListView", "LargeListView"]
                .iter()
                .any(|kind| {
                    actual == format!("{kind}({element})")
                        || actual.starts_with(&format!("{kind}({element},"))
                        || actual.starts_with(&format!("{kind}(non-null {element}"))
                })
        }
        _ => actual == canonical_arrow(expected),
    }
}

fn title_case(value: &str) -> String {
    let mut chars = value.chars();
    chars
        .next()
        .map(|first| first.to_ascii_uppercase().to_string() + chars.as_str())
        .unwrap_or_default()
}

pub(crate) async fn publish_run_results(
    repo: &RunRepo,
    artifacts: &ArtifactService,
    user: &CurrentUser,
    run_id: i32,
    max_file_bytes: u64,
    outputs: Vec<InspectedExecutionOutput>,
) -> Result<Vec<RunDataset>, RunError> {
    let mut staged_artifacts = Vec::new();
    let mut drafts = Vec::new();
    let mut datasets = Vec::new();
    for output in outputs {
        let metadata = output.metadata;
        let staged = artifacts
            .store()
            .stage(
                std::fs::File::open(&output.path).map_err(RunError::Io)?,
                StageLimits {
                    max_bytes: max_file_bytes,
                },
            )
            .map_err(ArtifactError::from)?;
        let size_bytes =
            i64::try_from(staged.size_bytes).map_err(|_| RunError::Invalid("结果过大".into()))?;
        drafts.push(ArtifactDraft {
            sha256: staged.sha256.clone(),
            size_bytes,
            storage_path: ArtifactStore::relative_path(&staged.sha256),
            format: "parquet".into(),
            media_type: Some("application/vnd.apache.parquet".into()),
            created_at_ms: now_ms(),
        });
        staged_artifacts.push(staged);
        let (schema_json, schema_hash) =
            crate::application::datasets::schema_json_and_hash(&metadata);
        datasets.push(NewRunDataset {
            artifact_id: 0,
            result_name: output.name,
            schema_json,
            schema_hash,
            row_count: metadata.row_count as i64,
            size_bytes: metadata.size_bytes as i64,
            created_at_ms: now_ms(),
        });
    }
    let store = artifacts.store().clone();
    let _publication_guard = artifacts.exclusive_publication_guard().await;
    let publish_store = store.clone();
    let published_new = Arc::new(Mutex::new(Vec::<String>::new()));
    let published_by_callback = published_new.clone();
    let publication = repo
        .complete_with_artifact_outputs(
            &user.id,
            RunArtifactOutputs {
                scope_id: user.storage_scope_id,
                run_id,
                drafts,
                datasets,
                finished_at_ms: now_ms(),
            },
            move |index, path| {
                let existed = publish_store
                    .resolve(path)
                    .map_err(|error| RepoError::Storage(error.to_string()))?
                    .is_file();
                publish_store
                    .publish(&mut staged_artifacts[index], path)
                    .map_err(|error| RepoError::Storage(error.to_string()))?;
                if !existed {
                    published_by_callback
                        .lock()
                        .expect("published run paths")
                        .push(path.to_string());
                }
                Ok(())
            },
        )
        .await;
    if publication.is_err() {
        for path in published_new.lock().expect("published run paths").iter() {
            let _ = store.remove(path);
        }
    }
    publication.map_err(Into::into)
}

fn valid_result_name(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(), Some('a'..='z' | 'A'..='Z' | '_'))
        && chars.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

#[cfg(test)]
mod tests {
    use super::valid_result_name;

    #[test]
    fn open_policy_result_names_are_stable_and_path_free() {
        assert!(valid_result_name("bus_voltage"));
        assert!(!valid_result_name("../bus"));
        assert!(!valid_result_name("bus-voltage"));
    }
}
