mod common;

use common::TestApp;

#[tokio::test]
async fn catalog_search_returns_all_matching_facades_with_user_visible_differences() {
    let app = TestApp::spawn().await;
    let matches = app.state.study_planning_service.search("潮流", None);

    assert!(matches.enabled.len() >= 2);
    assert!(
        matches
            .enabled
            .iter()
            .any(|facade| facade.function == "ds_radial_pf")
    );
    assert!(
        matches
            .enabled
            .iter()
            .any(|facade| facade.function == "ds_newton_raphson_pf")
    );
    assert!(
        matches
            .enabled
            .iter()
            .all(|facade| !facade.differences.is_empty())
    );
}

#[tokio::test]
async fn explicit_newton_is_not_replaced_by_radial() {
    let app = TestApp::spawn().await;
    let matches = app
        .state
        .study_planning_service
        .search("潮流", Some("ds_newton_raphson_pf"));

    assert_eq!(matches.enabled.len(), 1);
    assert_eq!(matches.enabled[0].function, "ds_newton_raphson_pf");
    assert!(matches.disabled.is_empty());
}

#[tokio::test]
async fn planning_lookup_is_driven_by_the_full_facade_catalog() {
    let app = TestApp::spawn().await;

    let state_estimation = app
        .state
        .study_planning_service
        .get("state_estimation_wls")
        .unwrap();
    assert_eq!(state_estimation.function, "ds_wls_se");
    assert!(
        state_estimation
            .inputs
            .iter()
            .any(|input| input.alias == "measurement")
    );
    assert_eq!(
        app.state.engine_snapshot.facades.enabled_count(),
        app.state.engine_snapshot.facades.catalog_entry_count()
    );
}
