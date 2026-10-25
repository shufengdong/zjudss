use std::collections::HashSet;

use zjudss::{api, build_router, contract_manifest};

#[test]
fn production_router_and_manifest_share_one_declaration() {
    let routes = api::route_manifest();
    let unique = routes
        .iter()
        .map(|route| (route.method, route.path))
        .collect::<HashSet<_>>();

    assert_eq!(routes.len(), 48);
    assert_eq!(unique.len(), 48);
    assert!(unique.contains(&("GET", "/health")));
    assert!(unique.contains(&("POST", "/api/auth/login")));
    assert!(unique.contains(&("POST", "/api/auth/logout")));
    assert!(unique.contains(&("POST", "/api/sessions/{id}/model-imports")));
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
    assert_eq!(manifest.tool_names.len(), 18);
    assert!(manifest.tool_names.contains(&"plan_study"));
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
}
