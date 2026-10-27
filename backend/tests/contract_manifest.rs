use std::collections::HashSet;

use zjudss::{api, build_router, contract_manifest};

#[test]
fn production_router_and_manifest_share_one_declaration() {
    let routes = api::route_manifest();
    let unique = routes
        .iter()
        .map(|route| (route.method, route.path))
        .collect::<HashSet<_>>();

    assert_eq!(routes.len(), 67);
    assert_eq!(unique.len(), 67);
    assert!(unique.contains(&("GET", "/health")));
    assert!(unique.contains(&("POST", "/api/auth/login")));
    assert!(unique.contains(&("POST", "/api/auth/logout")));
    assert!(unique.contains(&("POST", "/api/sessions/{id}/upload-batches")));
    assert!(unique.contains(&("GET", "/api/upload-batches/{id}")));
    assert!(unique.contains(&("PUT", "/api/upload-batches/{id}/parse-settings")));
    assert!(unique.contains(&("PUT", "/api/upload-batches/{id}/candidates")));
    assert!(unique.contains(&("DELETE", "/api/upload-batches/{id}")));
    assert!(unique.contains(&("POST", "/api/data-resources/{id}/replacement-batches")));
    for route in [
        ("GET", "/api/study-configurations/{id}"),
        ("DELETE", "/api/study-configurations/{id}"),
        ("PUT", "/api/study-configurations/{id}/facade"),
        ("PUT", "/api/study-configurations/{id}/bindings/{inputId}"),
        (
            "POST",
            "/api/study-configurations/{id}/bindings/{inputId}/mapping",
        ),
        ("PUT", "/api/study-configurations/{id}/structs/{inputId}"),
        ("PUT", "/api/study-configurations/{id}/results"),
    ] {
        assert!(unique.contains(&route));
    }
    for route in [
        ("POST", "/api/upload-batches/{id}/confirm"),
        ("GET", "/api/sessions/{id}/data-resources"),
        ("GET", "/api/data-resources/{id}"),
        ("PATCH", "/api/data-resources/{id}"),
        ("DELETE", "/api/data-resources/{id}"),
        ("GET", "/api/data-resources/{id}/rows"),
        ("GET", "/api/data-resources/{id}/download"),
        ("POST", "/api/data-resources/{id}/deletion-preview"),
        (
            "POST",
            "/api/data-resources/{id}/structured-replacement-previews",
        ),
        (
            "POST",
            "/api/data-resource-replacement-previews/{id}/confirm",
        ),
    ] {
        assert!(unique.contains(&route), "missing {route:?}");
    }
    assert!(
        !unique
            .iter()
            .any(|(_, path)| path.contains("model-imports"))
    );
    assert!(unique.contains(&("POST", "/api/sessions/{id}/scripts/text")));
    assert!(unique.contains(&("GET", "/api/run-comparisons/{id}")));
    for path in [
        "/api/model-dataframes/{id}",
        "/api/model-dataframes/{id}/rows",
        "/api/model-dataframes/{id}/download",
        "/api/draft-dataframes/{id}",
        "/api/draft-dataframes/{id}/rows",
        "/api/draft-dataframes/{id}/download",
    ] {
        assert!(unique.contains(&("GET", path)), "missing {path}");
    }

    let _ = build_router;
}

#[test]
fn contract_manifest_contains_all_public_enums() {
    let manifest = contract_manifest().expect("embedded contract manifest");

    assert_eq!(manifest.version, "v4");
    assert_eq!(
        manifest.changes,
        [
            "dynamic_study_facades",
            "bounded_engine_execution",
            "recoverable_upload_batches",
            "independent_data_resources",
            "managed_study_configurations"
        ]
    );

    assert_eq!(
        manifest.message_types,
        [
            "user",
            "assistant",
            "tool-call",
            "progress",
            "result",
            "attachment",
            "model-status",
            "comparison",
            "script",
        ]
    );
    assert_eq!(manifest.tool_names.len(), 23);
    assert!(manifest.tool_names.contains(&"search_study_facades"));
    assert!(manifest.tool_names.contains(&"prepare_study"));
    for tool in [
        "list_data_resources",
        "read_data_resource",
        "update_data_resource_alias",
        "promote_result_dataset",
    ] {
        assert!(manifest.tool_names.contains(&tool));
    }
    assert_eq!(
        manifest.skill_ids,
        [
            "comparison",
            "model-intake",
            "result-analysis",
            "rss-generation",
            "study-planning",
            "troubleshooting",
        ]
    );
    assert_eq!(
        manifest.upload_sources,
        [
            "dss",
            "dssbundle",
            "rssbundle",
            "zip",
            "csv",
            "excel",
            "parquet",
            "rss_script",
        ]
    );
    assert_eq!(
        manifest.visualization_kinds,
        ["line", "bar", "scatter", "heatmap", "boxplot", "histogram"]
    );
    assert!(manifest.stable_error_codes.contains(&"agent_queue_full"));
    assert!(manifest.stable_error_codes.contains(&"invalid_credentials"));
    assert!(manifest.stable_error_codes.contains(&"invalid_page"));
    for code in [
        "upload_file_too_large",
        "upload_batch_too_large",
        "archive_limits_exceeded",
        "upload_batch_expired",
        "upload_parse_settings_invalid",
        "excel_formula_value_missing",
    ] {
        assert!(
            manifest.stable_error_codes.contains(&code),
            "missing {code}"
        );
    }
    assert!(
        manifest
            .stable_error_codes
            .contains(&"dataframe_read_failed")
    );
    assert!(
        manifest
            .stable_error_codes
            .contains(&"authentication_required")
    );
    assert!(
        manifest
            .stable_error_codes
            .contains(&"active_model_changed")
    );
    assert!(
        manifest
            .stable_error_codes
            .contains(&"history_search_unavailable")
    );
    for code in [
        "skill_required",
        "study_catalog_invalid",
        "study_model_read_failed",
        "study_plan_required",
        "study_plan_not_found",
        "study_plan_stale",
        "study_plan_function_mismatch",
        "study_facades_missing",
        "facade_contract_invalid",
        "facade_schema_invalid",
        "facade_unsupported_input_kind",
        "engine_timeout",
        "result_monitor_failed",
        "result_file_limit",
        "result_file_too_large",
        "result_total_too_large",
        "result_symlink_rejected",
        "result_path_escape",
    ] {
        assert!(
            manifest.stable_error_codes.contains(&code),
            "missing {code}"
        );
    }
}
