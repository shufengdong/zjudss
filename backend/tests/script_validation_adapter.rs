use std::path::PathBuf;
use std::time::Duration;

use zjudss::adapters::{STATIC_MODEL_TABLES, ScriptValidationSpec, SimEngine, StubEngine};

async fn validate(source: &str) -> zjudss::adapters::StaticScriptValidation {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("script.rss"), source).unwrap();
    StubEngine
        .validate_script(ScriptValidationSpec {
            input_tree: root.path().to_path_buf(),
            script_path: PathBuf::from("script.rss"),
            timeout: Duration::from_secs(1),
        })
        .await
        .unwrap()
}

#[tokio::test]
async fn static_validation_extracts_required_aliases_and_named_outputs_without_a_model() {
    let result = validate(
        "[voltage, _, summary] = \
         ds_radial_pf(bus, line, linecode, load, transformer, voltage_source);\n",
    )
    .await;
    assert!(result.valid);
    assert_eq!(
        result.required_aliases,
        [
            "bus",
            "line",
            "linecode",
            "load",
            "transformer",
            "voltage_source"
        ]
    );
    assert_eq!(result.outputs, ["summary", "voltage"]);
    assert_eq!(result.called_functions, ["ds_radial_pf"]);
}

#[tokio::test]
async fn static_validation_accepts_the_eig_rs_single_result_destructuring_form() {
    let result = validate(
        "[voltage] = ds_radial_pf(\n\
             bus, line, linecode, load, transformer, voltage_source,\n\
             motor, transformer_winding, transformer_impedance, current_source,\n\
             generator, ibr, switch, shunt, capacitor, reactor, spectrum,\n\
             control_profile, xy_curve, storage, para\n\
         );",
    )
    .await;
    let mut expected_aliases = STATIC_MODEL_TABLES.map(str::to_string).to_vec();
    expected_aliases.sort();

    assert!(result.valid);
    assert_eq!(result.outputs, ["voltage"]);
    assert_eq!(result.required_aliases, expected_aliases);
    assert_eq!(result.called_functions, ["ds_radial_pf"]);
}

#[tokio::test]
async fn governed_call_detection_ignores_comments_strings_and_identifier_suffixes() {
    let result = validate(
        "# ds_fault_study(bus)\n\
         note = \"ds_harmonic_study(bus)\";\n\
         custom = my_ds_motor_dynamics(bus);\n\
         fault = ds_fault_study(bus);\n\
         voltage = ds_newton_raphson_pf(bus); // ds_radial_qsts(bus)\n",
    )
    .await;

    assert!(result.valid);
    assert_eq!(
        result.called_functions,
        ["ds_fault_study", "ds_newton_raphson_pf"]
    );
}

#[tokio::test]
async fn static_validation_rejects_imports_paths_urls_and_external_reads() {
    for source in [
        "x = import_dss(\"model.dss\");",
        "x = read(\"/tmp/data.csv\");",
        "x = read(\"https://example.invalid/data.parquet\");",
        "x = import_bmopf(\"model.json\");",
    ] {
        let result = validate(source).await;
        assert!(!result.valid, "{source}");
        assert_eq!(
            result.diagnostics["code"],
            "script_external_dependency_rejected"
        );
        assert!(result.called_functions.is_empty());
    }
}

#[tokio::test]
async fn static_validation_requires_a_named_dataframe_output() {
    let result = validate("print(1);").await;
    assert!(!result.valid);
    assert_eq!(result.diagnostics["code"], "script_output_required");
    assert!(result.called_functions.is_empty());
}
