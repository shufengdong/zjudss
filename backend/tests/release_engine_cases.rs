#![cfg(feature = "release-test")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use arrow_array::{Float64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use backend::adapters::{
    ENGINE_CONTRACT_VERSION, EngineConfig, ImportInspectSpec, ImportProfile, ImportSpec,
    NamedInput, ProcessEngine, RunMode, RunSpec, SimEngine,
};
use backend::engine_resources::{EngineExample, EngineResourceRegistry};
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde::Serialize;
use sha2::{Digest, Sha256};

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
    discovered_study_types: Vec<String>,
    contract_checks: ContractChecks,
    representative_runs: Vec<RunReport>,
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
    source_kind: String,
    success: bool,
    datasets: Vec<DatasetReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    failure_code: Option<String>,
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
        format!("{:x}", Sha256::digest(&bytes)),
        expected_hash,
        "release_engine_example_hash_mismatch"
    );
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(target, bytes).unwrap();
}

fn import_all_examples(
    snapshot: &backend::engine_resources::EngineResourceSnapshot,
    root: &Path,
) -> usize {
    let mut imported = 0;
    for example in &snapshot.examples {
        if !example.importable {
            continue;
        }
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

fn representative_examples(examples: &[EngineExample]) -> Vec<&EngineExample> {
    let mut selected = Vec::new();
    for study_type in [
        "execution_plan",
        "static_power_flow",
        "fault_study",
        "harmonic",
    ] {
        if let Some(example) = examples.iter().find(|example| {
            example.importable
                && example
                    .manifest
                    .as_ref()
                    .is_some_and(|manifest| manifest.study.kind == study_type)
        }) {
            selected.push(example);
        }
    }
    selected
}

fn prepare_run_tree(
    snapshot: &backend::engine_resources::EngineResourceSnapshot,
    example: &EngineExample,
    root: &Path,
) -> (PathBuf, PathBuf, RunMode) {
    let tree = root.join(format!("run-{}", example.id));
    std::fs::create_dir_all(&tree).unwrap();
    let bundle = tree.join("case.rssbundle");
    for file in &example.files {
        copy_file(
            &snapshot.engine_root.join(&file.source_path),
            &bundle.join(&file.virtual_path),
            &file.sha256,
        );
    }
    (tree, "case.rssbundle".into(), RunMode::Bundle)
}

async fn run_representative(
    engine: &ProcessEngine,
    snapshot: &backend::engine_resources::EngineResourceSnapshot,
    example: &EngineExample,
    study_type: &str,
    root: &Path,
) -> RunReport {
    let (tree, script_path, mode) = prepare_run_tree(snapshot, example, root);
    let spec = RunSpec {
        mode,
        input_tree: tree,
        script_path,
        named_inputs: vec![],
        result_dir: root.join(format!("results-{}", example.id)),
        timeout: Duration::from_secs(120),
    };
    let result = match mode {
        RunMode::Bundle => engine.run_bundle(spec).await,
        _ => engine.run_dataframe(spec).await,
    };
    match result {
        Ok(output) => RunReport {
            example_id: example.id.clone(),
            study_type: study_type.to_string(),
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
        },
        Err(error) => {
            eprintln!("release_engine_run_failed:{study_type}:{}", error.code);
            RunReport {
                example_id: example.id.clone(),
                study_type: study_type.to_string(),
                source_kind: example.source_kind.clone(),
                success: false,
                datasets: vec![],
                failure_code: Some(error.code),
            }
        }
    }
}

#[tokio::test]
async fn real_engine_imports_all_examples_and_runs_representative_studies() {
    let binary = required_path("TENSOREVAL_BIN");
    let cwd = required_path("TENSOREVAL_CWD");
    let config = EngineConfig::probe(binary.clone(), Duration::from_secs(120))
        .unwrap_or_else(|error| panic!("release_engine_capability_mismatch:{}", error.code));
    assert_eq!(
        config.capabilities.contract_version,
        ENGINE_CONTRACT_VERSION
    );
    let binary_sha256 = config.binary_sha256.clone();
    let resource_features = config.capabilities.resource_features();
    let engine = ProcessEngine::new(config);
    let snapshot = EngineResourceRegistry::load(&cwd, &resource_features)
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
    assert!(importable_examples > 0, "release_engine_importable_empty");
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

    let cases = representative_examples(&snapshot.examples);
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
    for example in cases {
        let study_type = &example.manifest.as_ref().unwrap().study.kind;
        representative_runs
            .push(run_representative(&engine, &snapshot, example, study_type, temp.path()).await);
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

    let failed: Vec<_> = report
        .representative_runs
        .iter()
        .filter(|run| !run.success)
        .map(|run| {
            format!(
                "{}:{}",
                run.study_type,
                run.failure_code.as_deref().unwrap_or("unknown")
            )
        })
        .collect();
    assert!(
        failed.is_empty(),
        "release_engine_representative_failed:{}",
        failed.join(",")
    );
}
