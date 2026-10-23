use std::collections::HashSet;

use backend::{api, build_router, contract_manifest};

#[test]
fn production_router_and_manifest_share_one_declaration() {
    let routes = api::route_manifest();
    let unique = routes
        .iter()
        .map(|route| (route.method, route.path))
        .collect::<HashSet<_>>();

    assert_eq!(routes.len(), 29);
    assert_eq!(unique.len(), 29);
    assert!(unique.contains(&("GET", "/health")));
    assert!(unique.contains(&("POST", "/api/auth/login")));
    assert!(unique.contains(&("POST", "/api/auth/logout")));
    assert!(unique.contains(&("POST", "/api/sessions/{id}/attachments")));
    assert!(unique.contains(&("GET", "/api/task-comparisons/{id}")));

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
        ]
    );
    assert_eq!(manifest.tool_names.len(), 22);
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
            "single_rss",
            "zip_rssbundle",
            "directory_rssbundle",
            "csv",
            "parquet",
            "excel",
        ]
    );
    assert_eq!(
        manifest.visualization_kinds,
        ["line", "bar", "scatter", "heatmap", "boxplot", "histogram"]
    );
    assert!(manifest.stable_error_codes.contains(&"agent_queue_full"));
    assert!(manifest.stable_error_codes.contains(&"invalid_credentials"));
    assert!(
        manifest
            .stable_error_codes
            .contains(&"authentication_required")
    );
    assert!(manifest.stable_error_codes.contains(&"version_conflict"));
    assert!(
        manifest
            .stable_error_codes
            .contains(&"history_search_unavailable")
    );
}
