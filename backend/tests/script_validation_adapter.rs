use zjudss::adapters::{ENGINE_CONTRACT_VERSION, EngineCapabilities};

fn capabilities() -> EngineCapabilities {
    EngineCapabilities {
        contract_version: ENGINE_CONTRACT_VERSION.into(),
        result_dir: true,
        output_format_parquet: true,
        named_parquet_inputs: true,
        static_dss_import: true,
        single_solve_import: true,
    }
}

#[test]
fn v4_runtime_capabilities_only_require_parquet_and_import_profiles() {
    let capabilities = capabilities();
    capabilities.require_v3().unwrap();
    assert_eq!(
        capabilities.resource_features(),
        [
            "named_parquet_inputs".to_string(),
            "single_solve_import".to_string(),
            "static_dss_import".to_string(),
        ]
        .into_iter()
        .collect()
    );
}

#[test]
fn missing_import_profile_still_fails_capability_probe() {
    let mut capabilities = capabilities();
    capabilities.single_solve_import = false;
    assert_eq!(
        capabilities.require_v3().unwrap_err().code,
        "engine_single_solve_import_unsupported"
    );
}
