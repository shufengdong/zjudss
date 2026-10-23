use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use arrow_array::{Float64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use backend::adapters::{
    ENGINE_CONTRACT_VERSION, EngineCapabilities, EngineConfig, NamedInput, OutputLimits, RunMode,
    RunSpec, build_command, validate_outputs,
};
#[cfg(feature = "release-test")]
use backend::adapters::{ProcessEngine, SimEngine};
use backend::storage::ParquetInspector;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;

fn write_parquet(path: &Path, rows: usize, compression: Compression) {
    let schema = Arc::new(Schema::new(vec![
        Field::new("bus", DataType::Utf8, false),
        Field::new("v_mag", DataType::Float64, false),
    ]));
    let names: Vec<_> = (0..rows).map(|index| format!("b{index}")).collect();
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(names)) as _,
            Arc::new(Float64Array::from(vec![1.0; rows])) as _,
        ],
    )
    .unwrap();
    let file = std::fs::File::create(path).unwrap();
    let props = WriterProperties::builder()
        .set_compression(compression)
        .build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(props)).unwrap();
    writer.write(&batch).unwrap();
    writer.close().unwrap();
}

fn tree() -> (tempfile::TempDir, PathBuf) {
    let root = tempfile::tempdir().unwrap();
    let input = root.path().join("task-input");
    std::fs::create_dir_all(input.join("case.rssbundle")).unwrap();
    std::fs::write(input.join("main.rss"), "result = input;\n").unwrap();
    std::fs::write(
        input.join("query.rss"),
        "result = select(col(bus), col(v_mag), input);\n",
    )
    .unwrap();
    write_parquet(&input.join("input.parquet"), 2, Compression::SNAPPY);
    (root, input)
}

fn spec(root: &Path, input_tree: &Path, mode: RunMode) -> RunSpec {
    let (script_path, named_inputs) = match mode {
        RunMode::Rss => (PathBuf::from("main.rss"), vec![]),
        RunMode::Bundle => (PathBuf::from("case.rssbundle"), vec![]),
        RunMode::Dataframe | RunMode::Analysis => (
            PathBuf::from("query.rss"),
            vec![NamedInput {
                alias: "input".into(),
                relative_path: "input.parquet".into(),
            }],
        ),
    };
    RunSpec {
        mode,
        input_tree: input_tree.to_path_buf(),
        script_path,
        named_inputs,
        result_dir: root.join(format!("result-{mode:?}")),
        timeout: Duration::from_secs(5),
    }
}

#[test]
fn four_commands_use_private_cwd_parquet_results_and_safe_named_inputs() {
    let (root, input_tree) = tree();
    for mode in [
        RunMode::Rss,
        RunMode::Bundle,
        RunMode::Dataframe,
        RunMode::Analysis,
    ] {
        let spec = spec(root.path(), &input_tree, mode);
        let command = build_command(Path::new("/opt/tensoreval"), &spec).unwrap();
        assert_eq!(command.cwd, input_tree);
        assert_eq!(command.program, Path::new("/opt/tensoreval"));
        let args: Vec<_> = command
            .args
            .iter()
            .map(|value| value.to_string_lossy().to_string())
            .collect();
        assert_eq!(
            args.iter().filter(|value| *value == "--result-dir").count(),
            1
        );
        assert!(
            args.windows(2)
                .any(|pair| pair == ["--output-format", "parquet"])
        );
        assert_eq!(
            args.first().map(String::as_str),
            if mode == RunMode::Bundle {
                Some("--result-dir")
            } else {
                Some("--df")
            }
        );
        if matches!(mode, RunMode::Dataframe | RunMode::Analysis) {
            assert!(
                args.windows(2)
                    .any(|pair| pair == ["--input", "input=input.parquet"])
            );
        }
    }

    let mut malicious = spec(root.path(), &input_tree, RunMode::Analysis);
    malicious.named_inputs[0].alias = "x; touch PWNED".into();
    assert_eq!(
        build_command(Path::new("tensoreval"), &malicious)
            .unwrap_err()
            .code,
        "input_alias_invalid"
    );
    assert!(!root.path().join("PWNED").exists());
}

#[test]
fn task_local_tree_survives_shared_example_removal() {
    let source = tempfile::tempdir().unwrap();
    std::fs::write(source.path().join("case.rss"), "result = 1;\n").unwrap();
    let task = tempfile::tempdir().unwrap();
    let tree = task.path().join("input");
    std::fs::create_dir(&tree).unwrap();
    std::fs::copy(source.path().join("case.rss"), tree.join("case.rss")).unwrap();
    drop(source);
    let spec = RunSpec {
        mode: RunMode::Rss,
        input_tree: tree.clone(),
        script_path: "case.rss".into(),
        named_inputs: vec![],
        result_dir: task.path().join("out"),
        timeout: Duration::from_secs(1),
    };
    let command = build_command(Path::new("tensoreval"), &spec).unwrap();
    assert_eq!(command.cwd, tree);
    assert!(command.cwd.join("case.rss").is_file());
}

#[test]
fn parquet_metadata_page_and_projection_preserve_schema_order_and_rows() {
    let root = tempfile::tempdir().unwrap();
    let path = root.path().join("voltage.parquet");
    write_parquet(&path, 3, Compression::SNAPPY);
    let metadata = ParquetInspector::metadata(&path).unwrap();
    assert_eq!(metadata.row_count, 3);
    assert_eq!(
        metadata
            .columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        ["bus", "v_mag"]
    );
    assert!(metadata.snappy);
    assert_eq!(
        ParquetInspector::page(&path, 1, 1)
            .unwrap()
            .iter()
            .map(RecordBatch::num_rows)
            .sum::<usize>(),
        1
    );
    let projected = ParquetInspector::projection(&path, &["v_mag".into()], 0, 2).unwrap();
    assert_eq!(projected[0].schema().fields().len(), 1);
    assert_eq!(projected[0].schema().field(0).name(), "v_mag");

    let empty = root.path().join("empty.parquet");
    write_parquet(&empty, 0, Compression::SNAPPY);
    let empty_metadata = ParquetInspector::metadata(&empty).unwrap();
    assert_eq!(empty_metadata.row_count, 0);
    assert!(
        empty_metadata.snappy,
        "没有数据页的空 Parquet 不应被误判为非 Snappy"
    );
}

#[test]
fn output_validation_rejects_format_names_corruption_duplicates_and_limits() {
    fn error_for(setup: impl FnOnce(&Path), limits: OutputLimits) -> String {
        let root = tempfile::tempdir().unwrap();
        setup(root.path());
        validate_outputs(root.path(), limits).unwrap_err().code
    }
    let defaults = OutputLimits::default();
    assert_eq!(
        error_for(
            |root| {
                std::fs::write(root.join("x.csv"), "x").unwrap();
            },
            defaults
        ),
        "result_format_invalid"
    );
    assert_eq!(
        error_for(
            |root| {
                write_parquet(&root.join("bad-name.parquet"), 1, Compression::SNAPPY);
            },
            defaults
        ),
        "result_name_invalid"
    );
    assert_eq!(
        error_for(
            |root| {
                std::fs::write(root.join("broken.parquet"), "PAR1bad").unwrap();
            },
            defaults
        ),
        "parquet_footer_invalid"
    );
    let duplicate_root = tempfile::tempdir().unwrap();
    write_parquet(
        &duplicate_root.path().join("A.parquet"),
        1,
        Compression::SNAPPY,
    );
    write_parquet(
        &duplicate_root.path().join("a.parquet"),
        1,
        Compression::SNAPPY,
    );
    if std::fs::read_dir(duplicate_root.path()).unwrap().count() == 2 {
        assert_eq!(
            validate_outputs(duplicate_root.path(), defaults)
                .unwrap_err()
                .code,
            "result_name_duplicate"
        );
    }
    assert_eq!(
        error_for(
            |root| {
                write_parquet(&root.join("one.parquet"), 1, Compression::SNAPPY);
                write_parquet(&root.join("two.parquet"), 1, Compression::SNAPPY);
            },
            OutputLimits {
                max_files: 1,
                ..defaults
            }
        ),
        "result_file_limit"
    );
    assert_eq!(
        error_for(
            |root| {
                write_parquet(&root.join("one.parquet"), 1, Compression::SNAPPY);
            },
            OutputLimits {
                max_file_bytes: 1,
                ..defaults
            }
        ),
        "result_file_too_large"
    );
    assert_eq!(
        error_for(
            |root| {
                write_parquet(&root.join("one.parquet"), 1, Compression::SNAPPY);
            },
            OutputLimits {
                max_total_bytes: 1,
                ..defaults
            }
        ),
        "result_total_too_large"
    );
    assert_eq!(
        error_for(
            |root| {
                write_parquet(&root.join("one.parquet"), 1, Compression::UNCOMPRESSED);
            },
            defaults
        ),
        "result_compression_invalid"
    );
}

#[cfg(unix)]
#[test]
fn output_validation_rejects_symlinks_and_capability_probe_fails_closed() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let root = tempfile::tempdir().unwrap();
    let outside = root.path().join("outside.parquet");
    write_parquet(&outside, 1, Compression::SNAPPY);
    let output = root.path().join("out");
    std::fs::create_dir(&output).unwrap();
    symlink(&outside, output.join("linked.parquet")).unwrap();
    assert_eq!(
        validate_outputs(&output, OutputLimits::default())
            .unwrap_err()
            .code,
        "result_symlink_rejected"
    );

    let good = root.path().join("good-engine");
    std::fs::write(
        &good,
        "#!/bin/sh\nif [ \"$1\" = \"--help\" ]; then echo 'Usage --result-dir --output-format parquet --input NAME=FILE'; exit 0; fi\nmkdir -p \"$5\"\ncp probe.parquet \"$5/result.parquet\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&good, std::fs::Permissions::from_mode(0o755)).unwrap();
    let capabilities = EngineCapabilities::probe(&good).unwrap();
    assert_eq!(capabilities.contract_version, ENGINE_CONTRACT_VERSION);
    capabilities.require_v2().unwrap();
    let mismatched = EngineCapabilities {
        contract_version: "legacy".into(),
        result_dir: true,
        output_format_parquet: true,
        named_parquet_inputs: true,
        static_dss_import: true,
        single_solve_import: true,
        script_validation: true,
    };
    assert_eq!(
        mismatched.require_v2().unwrap_err().code,
        "engine_contract_mismatch"
    );

    let old = root.path().join("old-engine");
    std::fs::write(
        &old,
        "#!/bin/sh\necho 'Usage --result-dir --input NAME=FILE'\n",
    )
    .unwrap();
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
    let error = EngineConfig::probe(old, Duration::from_secs(1)).unwrap_err();
    assert_eq!(error.code, "engine_parquet_output_unsupported");
}

#[cfg(unix)]
#[test]
fn release_script_fails_closed_with_stable_preflight_reasons() {
    use std::os::unix::fs::PermissionsExt;

    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .join("scripts/test-release-engine");
    let missing = Command::new(&script)
        .env_remove("TENSOREVAL_BIN")
        .env_remove("TENSOREVAL_CWD")
        .output()
        .unwrap();
    assert!(!missing.status.success());
    assert_eq!(
        String::from_utf8_lossy(&missing.stderr).trim(),
        "release_engine_missing:TENSOREVAL_BIN"
    );

    let root = tempfile::tempdir().unwrap();
    let old = root.path().join("old-tensoreval");
    std::fs::write(&old, "#!/bin/sh\necho 'Usage: tensoreval --result-dir'\n").unwrap();
    std::fs::set_permissions(&old, std::fs::Permissions::from_mode(0o755)).unwrap();
    let incompatible = Command::new(&script)
        .env("TENSOREVAL_BIN", &old)
        .env("TENSOREVAL_CWD", root.path())
        .output()
        .unwrap();
    assert!(!incompatible.status.success());
    assert_eq!(
        String::from_utf8_lossy(&incompatible.stderr).trim(),
        "release_engine_capability_mismatch:output_format_parquet"
    );
}

#[cfg(feature = "release-test")]
#[tokio::test]
async fn real_tensoreval_smoke_writes_snappy_parquet() {
    let binary = PathBuf::from(std::env::var_os("TENSOREVAL_BIN").expect("set TENSOREVAL_BIN"));
    let engine = ProcessEngine::new(EngineConfig::probe(binary, Duration::from_secs(30)).unwrap());
    let (root, input_tree) = tree();
    let output = engine
        .run_analysis(spec(root.path(), &input_tree, RunMode::Analysis))
        .await
        .unwrap();
    assert_eq!(output.datasets.len(), 1);
    assert!(
        output
            .datasets
            .iter()
            .all(|dataset| dataset.metadata.snappy)
    );
}
