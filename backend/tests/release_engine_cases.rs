#![cfg(feature = "release-test")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arrow_array::{Float64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde::Serialize;
use sha2::{Digest, Sha256};
use zjudss::adapters::{
    AdapterError, CaseProjectionSpec, ENGINE_CONTRACT_VERSION, EngineConfig, ErrorCategory,
    ImportInspectSpec, ImportProfile, ImportSpec, NamedInput, ProcessEngine, ProjectionOutputSpec,
    RunMode, RunSpec, SimEngine,
};
use zjudss::application::CaseImportService;
use zjudss::engine_resources::{
    EngineExample, EngineResourceRegistry, FacadeInputKind, ProjectorOutputDisposition,
};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ReleaseReport {
    schema_version: u32,
    engine_contract: String,
    engine_resource_snapshot: String,
    binary_sha256: String,
    bundle_examples: usize,
    importable_examples: usize,
    non_importable_examples: usize,
    non_importable_details: Vec<NonImportableExample>,
    facade_catalog_entries: usize,
    enabled_facades: usize,
    facade_diagnostics: Vec<String>,
    projection_catalog_entries: usize,
    enabled_projection_profiles: usize,
    referenced_projection_profiles: usize,
    projection_diagnostics: Vec<String>,
    projected_examples: usize,
    projection_failures: usize,
    case_projections: Vec<CaseProjectionReport>,
    discovered_study_types: Vec<String>,
    contract_checks: ContractChecks,
    representative_runs: Vec<RunReport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseProjectionReport {
    example_id: String,
    projection_profile: String,
    success: bool,
    model_outputs: usize,
    data_outputs: usize,
    struct_outputs: usize,
    diagnostic_outputs: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_code: Option<String>,
    #[serde(skip)]
    failure_message: Option<String>,
}

struct PreparedCaseProjection {
    tree: PathBuf,
    named_inputs: Vec<NamedInput>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NonImportableExample {
    example_id: String,
    diagnostics: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ContractChecks {
    static_model_import: bool,
    static_import_dataframes: usize,
    single_solve_model_import: bool,
    named_parquet_input: bool,
    result_dir: bool,
    output_format_parquet: bool,
    expr_df: bool,
    snappy: bool,
    multiple_results: bool,
    failure_diagnostics: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct RunReport {
    example_id: String,
    study_type: String,
    projection_profile: String,
    source_kind: String,
    success: bool,
    datasets: Vec<DatasetReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_code: Option<String>,
    #[serde(skip)]
    failure_message: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct DatasetReport {
    name: String,
    row_count: u64,
    codec: &'static str,
    columns: Vec<ColumnReport>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ColumnReport {
    name: String,
    dtype: String,
    nullable: bool,
    unit: Option<String>,
}

fn required_path(name: &str) -> PathBuf {
    let value = std::env::var_os(name).unwrap_or_else(|| panic!("release_engine_missing:{name}"));
    PathBuf::from(value)
        .canonicalize()
        .unwrap_or_else(|_| panic!("release_engine_path_invalid:{name}"))
}

fn copy_file(source: &Path, target: &Path, expected_hash: &str) {
    let bytes = std::fs::read(source).expect("read snapshot example file");
    assert_eq!(
        hex::encode(Sha256::digest(&bytes)),
        expected_hash,
        "release_engine_example_hash_mismatch"
    );
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, bytes).unwrap();
}

fn import_all_examples(
    snapshot: &zjudss::engine_resources::EngineResourceSnapshot,
    root: &Path,
) -> usize {
    let mut imported = 0;
    for example in &snapshot.examples {
        if !example.importable {
            continue;
        }
        assert!(
            CaseImportService::is_modelizable(example),
            "release_engine_importable_case_not_modelizable:{}",
            example.id
        );
        let scope = root.join(&example.id);
        for file in &example.files {
            copy_file(
                &snapshot.engine_root.join(&file.source_path),
                &scope.join(&file.virtual_path),
                &file.sha256,
            );
        }
        assert!(
            scope.is_dir(),
            "release_engine_import_scope_empty:{}",
            example.id
        );
        imported += 1;
    }
    imported
}

fn write_named_input(path: &Path) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("bus", DataType::Utf8, false),
        Field::new("v_mag", DataType::Float64, false),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec!["source", "load"])) as _,
            Arc::new(Float64Array::from(vec![1.0, 0.98])) as _,
        ],
    )
    .unwrap();
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    let mut writer = ArrowWriter::try_new(
        std::fs::File::create(path).unwrap(),
        schema,
        Some(properties),
    )
    .unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

async fn verify_model_import_profiles(engine: &ProcessEngine, root: &Path) -> (bool, bool) {
    let base = "New Circuit.release basekv=10\n";
    for (name, profile, suffix) in [
        ("static", ImportProfile::StaticDss, ""),
        (
            "single-solve",
            ImportProfile::SingleSolveExecution,
            "\nSolve\n",
        ),
    ] {
        let input_tree = root.join(format!("import-{name}"));
        std::fs::create_dir_all(&input_tree).unwrap();
        std::fs::write(input_tree.join("model.dss"), format!("{base}{suffix}")).unwrap();
        let inspection = engine
            .inspect_model_import(ImportInspectSpec {
                input_tree: input_tree.clone(),
                timeout: Duration::from_secs(30),
            })
            .await
            .unwrap_or_else(|error| panic!("release_engine_import_inspect_failed:{}", error.code));
        assert_eq!(inspection.entry_candidates.len(), 1);
        let imported = engine
            .import_model(ImportSpec {
                input_tree,
                entry_candidate_id: inspection.entry_candidates[0].id.clone(),
                result_dir: root.join(format!("import-{name}-results")),
                timeout: Duration::from_secs(120),
            })
            .await
            .unwrap_or_else(|error| {
                panic!("release_engine_model_import_failed:{name}:{}", error.code)
            });
        assert_eq!(imported.profile, profile);
        assert_eq!(imported.dataframes.len(), 21);
        assert!(
            imported
                .dataframes
                .iter()
                .all(|dataset| dataset.metadata.snappy)
        );
    }
    (true, true)
}

async fn verify_expr_df(
    engine: &ProcessEngine,
    root: &Path,
    static_model_import: bool,
    single_solve_model_import: bool,
) -> ContractChecks {
    let tree = root.join("expr-df");
    std::fs::create_dir_all(&tree).unwrap();
    write_named_input(&tree.join("input.parquet"));
    std::fs::write(
        tree.join("multi.rss"),
        "one = select(col(bus), col(v_mag), input);\ntwo = select(col(bus), col(v_mag), input);\n",
    )
    .unwrap();
    let output = engine
        .run_analysis(RunSpec {
            mode: RunMode::Analysis,
            input_tree: tree.clone(),
            script_path: "multi.rss".into(),
            named_inputs: vec![NamedInput {
                alias: "input".into(),
                relative_path: "input.parquet".into(),
            }],
            result_dir: root.join("expr-df-results"),
            timeout: Duration::from_secs(30),
        })
        .await
        .unwrap_or_else(|error| panic!("release_engine_expr_df_failed:{}", error.code));
    assert_eq!(
        output.datasets.len(),
        2,
        "release_engine_multi_result_failed"
    );
    assert!(
        output
            .datasets
            .iter()
            .all(|dataset| dataset.metadata.snappy)
    );

    std::fs::write(
        tree.join("invalid.rss"),
        "result = no_such_function(input);\n",
    )
    .unwrap();
    let failure = engine
        .run_analysis(RunSpec {
            mode: RunMode::Analysis,
            input_tree: tree,
            script_path: "invalid.rss".into(),
            named_inputs: vec![NamedInput {
                alias: "input".into(),
                relative_path: "input.parquet".into(),
            }],
            result_dir: root.join("invalid-results"),
            timeout: Duration::from_secs(30),
        })
        .await
        .expect_err("invalid expr_df must fail");
    assert!(!failure.code.is_empty());
    assert!(!failure.message.trim().is_empty());

    ContractChecks {
        static_model_import,
        static_import_dataframes: 21,
        single_solve_model_import,
        named_parquet_input: true,
        result_dir: true,
        output_format_parquet: true,
        expr_df: true,
        snappy: true,
        multiple_results: true,
        failure_diagnostics: true,
    }
}

fn representative_examples(
    examples: &[EngineExample],
) -> std::collections::BTreeMap<&str, Vec<&EngineExample>> {
    let discovered = examples
        .iter()
        .filter_map(|example| {
            example
                .manifest
                .as_ref()
                .map(|manifest| manifest.projection.profile.as_str())
        })
        .collect::<std::collections::BTreeSet<_>>();
    let mut selected = std::collections::BTreeMap::<_, Vec<_>>::new();
    for example in examples.iter().filter(|example| example.importable) {
        let Some(manifest) = example.manifest.as_ref() else {
            continue;
        };
        selected
            .entry(manifest.projection.profile.as_str())
            .or_default()
            .push(example);
    }
    let missing = discovered
        .iter()
        .filter(|profile| !selected.contains_key(**profile))
        .copied()
        .collect::<Vec<_>>();
    assert!(
        missing.is_empty(),
        "release_engine_projection_profile_without_importable_example:{}",
        missing.join(",")
    );
    for candidates in selected.values_mut() {
        candidates.sort_by_key(|example| (example.size_bytes, example.file_count, &example.id));
    }
    selected
}

#[test]
fn selects_only_the_first_successfully_projected_representative_candidate() {
    let candidates = ["missing", "first", "second"];
    let available = std::collections::BTreeSet::from(["first", "second"]);

    assert_eq!(
        first_available_candidate(&candidates, |candidate| available.contains(candidate)),
        Some("first")
    );
}

fn first_available_candidate<'a>(
    candidates: &'a [&'a str],
    mut is_available: impl FnMut(&str) -> bool,
) -> Option<&'a str> {
    for candidate in candidates {
        if is_available(candidate) {
            return Some(*candidate);
        }
    }
    None
}

fn prepare_run_tree(
    snapshot: &zjudss::engine_resources::EngineResourceSnapshot,
    example: &EngineExample,
    root: &Path,
) -> PathBuf {
    let tree = root.join(format!("run-{}.rssbundle", example.id));
    std::fs::create_dir_all(&tree).unwrap();
    for file in &example.files {
        copy_file(
            &snapshot.engine_root.join(&file.source_path),
            &tree.join(&file.virtual_path),
            &file.sha256,
        );
    }
    tree
}

fn failed_projection(
    example: &EngineExample,
    projection_profile: &str,
    code: impl Into<String>,
    message: impl Into<String>,
) -> (CaseProjectionReport, Option<PreparedCaseProjection>) {
    (
        CaseProjectionReport {
            example_id: example.id.clone(),
            projection_profile: projection_profile.to_string(),
            success: false,
            model_outputs: 0,
            data_outputs: 0,
            struct_outputs: 0,
            diagnostic_outputs: 0,
            failure_code: Some(code.into()),
            failure_message: Some(message.into().replace(['\n', '\r'], " ")),
        },
        None,
    )
}

async fn project_example(
    engine: &ProcessEngine,
    snapshot: &zjudss::engine_resources::EngineResourceSnapshot,
    example: &EngineExample,
    root: &Path,
) -> (CaseProjectionReport, Option<PreparedCaseProjection>) {
    let manifest = example.manifest.as_ref().expect("importable manifest");
    let projection_profile = manifest.projection.profile.as_str();
    let profile = snapshot
        .case_projections
        .get_enabled(projection_profile)
        .expect("importable projection profile");
    let tree = prepare_run_tree(snapshot, example, root);
    let projected = match engine
        .project_case(CaseProjectionSpec {
            input_tree: tree.clone(),
            source_entry: manifest.source.entry.clone().into(),
            projector: profile.projector.clone(),
            outputs: profile
                .projector_outputs
                .iter()
                .map(|output| ProjectionOutputSpec {
                    alias: output.alias.clone(),
                    diagnostic: output.disposition == ProjectorOutputDisposition::Diagnostic,
                })
                .collect(),
            result_dir: root.join(format!("projection-results-{}", example.id)),
            timeout: Duration::from_secs(120),
        })
        .await
    {
        Ok(projected) => projected,
        Err(error) => {
            return failed_projection(example, projection_profile, error.code, error.message);
        }
    };

    let inputs = tree.join("inputs");
    if let Err(error) = std::fs::create_dir_all(&inputs) {
        return failed_projection(
            example,
            projection_profile,
            "case_projection_input_copy_failed",
            error.to_string(),
        );
    }
    let mut model_outputs = 0;
    let mut data_outputs = 0;
    let mut struct_outputs = 0;
    let mut named_inputs = Vec::with_capacity(projected.dataframes.len());
    for dataset in projected.dataframes {
        let contract = profile
            .projector_outputs
            .iter()
            .find(|output| {
                output.alias == dataset.name
                    && output.disposition == ProjectorOutputDisposition::FacadeInput
            })
            .expect("adapter returned a validated facade input");
        match contract.input_kind {
            FacadeInputKind::Model => model_outputs += 1,
            FacadeInputKind::Data => data_outputs += 1,
            FacadeInputKind::Struct => struct_outputs += 1,
        }
        let relative = PathBuf::from("inputs").join(format!("{}.parquet", dataset.name));
        if let Err(error) = std::fs::copy(&dataset.path, tree.join(&relative)) {
            return failed_projection(
                example,
                projection_profile,
                "case_projection_input_copy_failed",
                error.to_string(),
            );
        }
        named_inputs.push(NamedInput {
            alias: dataset.name,
            relative_path: relative,
        });
    }
    let diagnostic_outputs = projected.diagnostics.len();
    (
        CaseProjectionReport {
            example_id: example.id.clone(),
            projection_profile: projection_profile.to_string(),
            success: true,
            model_outputs,
            data_outputs,
            struct_outputs,
            diagnostic_outputs,
            failure_code: None,
            failure_message: None,
        },
        Some(PreparedCaseProjection { tree, named_inputs }),
    )
}

async fn inject_manifest_result_selector(
    engine: &ProcessEngine,
    tree: &Path,
    root: &Path,
    example: &EngineExample,
    profile: &zjudss::engine_resources::CaseProjectionProfile,
) -> Result<(), AdapterError> {
    let manifest = example.manifest.as_ref().expect("importable manifest");
    let selector = &profile.result_selector;
    let selector_input = profile
        .inputs
        .iter()
        .find(|input| input.id == selector.input)
        .expect("validated result selector input");
    let selected = manifest
        .projection
        .results
        .iter()
        .map(|result| result.target.clone())
        .collect::<Vec<_>>()
        .join(",");
    let selected_literal = serde_json::to_string(&selected).map_err(|error| {
        AdapterError::new(
            "case_projection_output_mismatch",
            ErrorCategory::Internal,
            error.to_string(),
        )
    })?;
    let script_name = "release-result-selector.rss";
    std::fs::write(
        tree.join(script_name),
        format!(
            "selected_input = with_columns(alias({selected_literal}, {}), {});\n",
            selector.field, selector_input.alias
        ),
    )
    .map_err(|error| {
        AdapterError::new(
            "case_projection_selector_write_failed",
            ErrorCategory::Io,
            error.to_string(),
        )
    })?;
    let relative_input = PathBuf::from("inputs").join(format!("{}.parquet", selector_input.alias));
    let output = engine
        .run_dataframe(RunSpec {
            mode: RunMode::Dataframe,
            input_tree: tree.to_path_buf(),
            script_path: script_name.into(),
            named_inputs: vec![NamedInput {
                alias: selector_input.alias.clone(),
                relative_path: relative_input.clone(),
            }],
            result_dir: root.join(format!("selector-results-{}", example.id)),
            timeout: Duration::from_secs(30),
        })
        .await?;
    let selected_input = output
        .datasets
        .into_iter()
        .find(|dataset| dataset.name == "selected_input")
        .ok_or_else(|| {
            AdapterError::new(
                "case_projection_output_mismatch",
                ErrorCategory::Model,
                "结果选择器没有生成 Struct 输入",
            )
        })?;
    std::fs::copy(selected_input.path, tree.join(relative_input)).map_err(|error| {
        AdapterError::new(
            "case_projection_selector_write_failed",
            ErrorCategory::Io,
            error.to_string(),
        )
    })?;
    Ok(())
}

async fn run_representative(
    engine: &ProcessEngine,
    example: &EngineExample,
    projection_profile: &str,
    profile: &zjudss::engine_resources::CaseProjectionProfile,
    prepared: &PreparedCaseProjection,
    root: &Path,
) -> RunReport {
    let tree = prepared.tree.clone();
    let manifest = example.manifest.as_ref().expect("importable manifest");
    if let Err(error) = inject_manifest_result_selector(engine, &tree, root, example, profile).await
    {
        return RunReport {
            example_id: example.id.clone(),
            study_type: manifest.study.kind.clone(),
            projection_profile: projection_profile.to_string(),
            source_kind: example.source_kind.clone(),
            success: false,
            datasets: vec![],
            failure_code: Some(error.code),
            failure_message: Some(error.message.replace(['\n', '\r'], " ")),
        };
    }
    let spec = RunSpec {
        mode: RunMode::Dataframe,
        input_tree: tree,
        script_path: manifest.projection.managed_entry.clone().into(),
        named_inputs: prepared.named_inputs.clone(),
        result_dir: root.join(format!("results-{}", example.id)),
        timeout: Duration::from_secs(120),
    };
    let result = engine.run_dataframe(spec).await;
    match result {
        Ok(output) => RunReport {
            example_id: example.id.clone(),
            study_type: manifest.study.kind.clone(),
            projection_profile: projection_profile.to_string(),
            source_kind: example.source_kind.clone(),
            success: true,
            datasets: output
                .datasets
                .into_iter()
                .map(|dataset| DatasetReport {
                    name: dataset.name,
                    row_count: dataset.metadata.row_count,
                    codec: "snappy",
                    columns: dataset
                        .metadata
                        .columns
                        .into_iter()
                        .map(|column| ColumnReport {
                            name: column.name,
                            dtype: column.dtype,
                            nullable: column.nullable,
                            unit: column.unit,
                        })
                        .collect(),
                })
                .collect(),
            failure_code: None,
            failure_message: None,
        },
        Err(error) => RunReport {
            example_id: example.id.clone(),
            study_type: manifest.study.kind.clone(),
            projection_profile: projection_profile.to_string(),
            source_kind: example.source_kind.clone(),
            success: false,
            datasets: vec![],
            failure_code: Some(error.code),
            failure_message: Some(error.message.replace(['\n', '\r'], " ")),
        },
    }
}

#[tokio::test]
async fn real_engine_projects_all_examples_and_runs_representative_studies() {
    let binary = required_path("TENSOREVAL_BIN");
    let sparrowzz_dir = required_path("SPARROWZZ_DIR");
    let config = EngineConfig::probe(binary.clone(), Duration::from_secs(120))
        .unwrap_or_else(|error| panic!("release_engine_capability_mismatch:{}", error.code));
    assert_eq!(
        config.capabilities.contract_version,
        ENGINE_CONTRACT_VERSION
    );
    let binary_sha256 = config.binary_sha256.clone();
    let resource_features = config.capabilities.resource_features();
    let engine = ProcessEngine::new(config);
    let snapshot = EngineResourceRegistry::load(&sparrowzz_dir, &resource_features)
        .unwrap_or_else(|error| panic!("release_engine_snapshot_invalid:{}", error.code()));
    let temp = tempfile::tempdir().unwrap();
    let (snapshot, _) = EngineResourceRegistry::publish(snapshot, temp.path())
        .unwrap_or_else(|error| panic!("release_engine_publish_invalid:{}", error.code()));
    let bundle_examples = snapshot.examples.len();
    assert!(bundle_examples > 0, "release_engine_bundle_empty");
    assert!(
        snapshot
            .examples
            .iter()
            .all(|example| example.source_kind == "rssbundle"),
        "release_engine_non_bundle_example_discovered"
    );
    let importable_examples = snapshot
        .examples
        .iter()
        .filter(|example| example.importable)
        .count();
    let non_importable_examples = bundle_examples - importable_examples;
    let facade_catalog_entries = snapshot.facades.catalog_entry_count();
    let enabled_facades = snapshot.facades.enabled_count();
    let facade_diagnostics = snapshot
        .facades
        .diagnostics()
        .into_iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect::<Vec<_>>();
    assert!(
        facade_catalog_entries > 0,
        "release_engine_facade_catalog_empty"
    );
    assert_eq!(
        enabled_facades, facade_catalog_entries,
        "release_engine_facade_disabled"
    );
    assert!(
        facade_diagnostics.is_empty(),
        "release_engine_facade_diagnostics"
    );
    let projection_catalog_entries = snapshot.case_projections.catalog_entry_count();
    let enabled_projection_profiles = snapshot.case_projections.enabled_count();
    let projection_diagnostics = snapshot
        .case_projections
        .diagnostics()
        .into_iter()
        .map(|diagnostic| diagnostic.code.clone())
        .collect::<Vec<_>>();
    assert!(
        projection_catalog_entries > 0,
        "release_engine_projection_catalog_empty"
    );
    assert_eq!(
        enabled_projection_profiles + projection_diagnostics.len(),
        projection_catalog_entries,
        "release_engine_projection_catalog_incomplete"
    );
    assert!(importable_examples > 0, "release_engine_importable_empty");
    assert_eq!(
        non_importable_examples, 0,
        "release_engine_manifest_projection_gap"
    );
    let importable = import_all_examples(&snapshot, &temp.path().join("imports"));
    assert_eq!(
        importable, importable_examples,
        "release_engine_importable_inventory_incomplete"
    );
    let (static_model_import, single_solve_model_import) =
        verify_model_import_profiles(&engine, temp.path()).await;
    let contract_checks = verify_expr_df(
        &engine,
        temp.path(),
        static_model_import,
        single_solve_model_import,
    )
    .await;

    let mut projection_candidates = snapshot
        .examples
        .iter()
        .filter(|example| example.importable)
        .collect::<Vec<_>>();
    projection_candidates.sort_by_key(|example| &example.id);
    let mut case_projections = Vec::with_capacity(projection_candidates.len());
    let mut prepared_projections = std::collections::BTreeMap::new();
    for example in projection_candidates {
        let (projection, prepared) =
            project_example(&engine, &snapshot, example, temp.path()).await;
        if let Some(prepared) = prepared {
            prepared_projections.insert(example.id.clone(), prepared);
        }
        case_projections.push(projection);
    }
    let projected_examples = case_projections
        .iter()
        .filter(|projection| projection.success)
        .count();
    let projection_failures = case_projections.len() - projected_examples;

    let cases = representative_examples(&snapshot.examples);
    let referenced_projection_profiles = cases.len();
    let discovered_study_types = snapshot
        .examples
        .iter()
        .filter_map(|example| {
            example
                .manifest
                .as_ref()
                .map(|manifest| manifest.study.kind.clone())
        })
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut representative_runs = Vec::new();
    for (projection_profile, candidates) in cases {
        let profile = snapshot
            .case_projections
            .get_enabled(projection_profile)
            .expect("importable profile");
        let candidate_ids = candidates
            .iter()
            .map(|example| example.id.as_str())
            .collect::<Vec<_>>();
        let representative = if let Some(example_id) =
            first_available_candidate(&candidate_ids, |candidate| {
                prepared_projections.contains_key(candidate)
            }) {
            let example = candidates
                .iter()
                .copied()
                .find(|example| example.id == example_id)
                .expect("selected representative candidate exists");
            let prepared = prepared_projections
                .get(example_id)
                .expect("selected representative projection exists");
            Some(
                run_representative(
                    &engine,
                    example,
                    projection_profile,
                    profile,
                    prepared,
                    temp.path(),
                )
                .await,
            )
        } else {
            None
        };
        representative_runs.push(representative.unwrap_or_else(|| {
            let example = candidates
                .first()
                .expect("study family has importable candidates");
            let manifest = example.manifest.as_ref().expect("importable manifest");
            RunReport {
                example_id: example.id.clone(),
                study_type: manifest.study.kind.clone(),
                projection_profile: projection_profile.to_string(),
                source_kind: example.source_kind.clone(),
                success: false,
                datasets: vec![],
                failure_code: Some("case_projection_failed".to_string()),
                failure_message: Some(
                    "该 profile 没有投影成功的代表案例，未执行 managed.rss".to_string(),
                ),
            }
        }));
    }

    let report = ReleaseReport {
        schema_version: 1,
        engine_contract: snapshot.contract_version.clone(),
        engine_resource_snapshot: snapshot.id.clone(),
        binary_sha256,
        bundle_examples,
        importable_examples,
        non_importable_examples,
        non_importable_details: snapshot
            .examples
            .iter()
            .filter(|example| !example.importable)
            .map(|example| NonImportableExample {
                example_id: example.id.clone(),
                diagnostics: example.diagnostics.clone(),
            })
            .collect(),
        facade_catalog_entries,
        enabled_facades,
        facade_diagnostics,
        projection_catalog_entries,
        enabled_projection_profiles,
        referenced_projection_profiles,
        projection_diagnostics,
        projected_examples,
        projection_failures,
        case_projections,
        discovered_study_types,
        contract_checks,
        representative_runs,
    };
    let report_path = std::env::var_os("RELEASE_ENGINE_REPORT")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .parent()
                .unwrap()
                .join("target/release-integration/engine-report.json")
        });
    std::fs::create_dir_all(report_path.parent().unwrap()).unwrap();
    std::fs::write(&report_path, serde_json::to_vec_pretty(&report).unwrap()).unwrap();

    let projection_ids = report
        .case_projections
        .iter()
        .map(|projection| projection.example_id.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        report.projected_examples, importable_examples,
        "release_engine_projected_inventory_incomplete"
    );
    assert_eq!(
        projection_ids.len(),
        importable_examples,
        "release_engine_projection_evidence_incomplete"
    );
    let failed_projections = report
        .case_projections
        .iter()
        .filter(|projection| !projection.success)
        .map(|projection| {
            format!(
                "{}:{}:{}",
                projection.example_id,
                projection.failure_code.as_deref().unwrap_or("unknown"),
                projection
                    .failure_message
                    .as_deref()
                    .unwrap_or("no message")
            )
        })
        .collect::<Vec<_>>();
    assert!(
        failed_projections.is_empty() && report.projection_failures == 0,
        "release_engine_projection_failed:{}",
        failed_projections.join(",")
    );

    let failed: Vec<_> = report
        .representative_runs
        .iter()
        .filter(|run| !run.success)
        .map(|run| {
            format!(
                "{}:{}:{}",
                run.study_type,
                run.failure_code.as_deref().unwrap_or("unknown"),
                run.failure_message.as_deref().unwrap_or("no message")
            )
        })
        .collect();
    assert!(
        failed.is_empty(),
        "release_engine_representative_failed:{}",
        failed.join(",")
    );
}
