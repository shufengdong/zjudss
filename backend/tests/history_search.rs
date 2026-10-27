mod common;
use common::TestApp;
use zjudss::{
    auth::CurrentUser,
    db::{models::NewMessage, repositories::SearchFilters},
};
fn p(key: &str) -> CurrentUser {
    common::test_user(key)
}
async fn message(app: &TestApp, p: &CurrentUser, session: i32, text: &str, at: i64) -> i32 {
    app.state
        .message_repo
        .create(
            &p.id,
            NewMessage {
                session_id: session,
                run_id: None,
                role: "user".into(),
                kind: "text".into(),
                content: text.into(),
                payload_json: None,
                created_at_ms: at,
            },
        )
        .await
        .unwrap()
        .id
}

#[tokio::test]
async fn default_session_title_is_meaningful_and_first_message_still_replaces_it() {
    let app = TestApp::spawn().await;
    let owner = p("instance");
    let session = app
        .state
        .session_repo
        .create(&owner.id, "  ", 1)
        .await
        .unwrap();
    assert_eq!(session.title, "新会话");
    assert_eq!(session.title_source, "default");

    message(&app, &owner, session.id, "检查馈线电压", 2).await;
    let renamed = app
        .state
        .session_repo
        .find(&owner.id, session.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.title, "检查馈线电压");
    assert_eq!(renamed.title_source, "auto");
}

#[tokio::test]
async fn chinese_substring_short_query_filters_delete_and_scope_are_bounded() {
    let app = TestApp::spawn().await;
    let owner = p("instance");
    let s = app
        .state
        .session_repo
        .create(&owner.id, "", 1)
        .await
        .unwrap();
    message(&app, &owner, s.id, "请分析配电网电压越限情况", 10).await;
    let found = app
        .state
        .history_service
        .search_sessions(
            &owner,
            "电网电压".into(),
            SearchFilters {
                script_name: None,
                status: None,
                from_ms: None,
                to_ms: None,
            },
            20,
        )
        .await
        .unwrap();
    assert_eq!(found[0].session_id, s.id.to_string());
    let short = app
        .state
        .history_service
        .search_sessions(
            &owner,
            "电压".into(),
            SearchFilters {
                script_name: None,
                status: None,
                from_ms: None,
                to_ms: None,
            },
            20,
        )
        .await
        .unwrap();
    assert_eq!(short.len(), 1);
    let renamed = app
        .state
        .session_repo
        .find(&owner.id, s.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.title_source, "auto");
    assert!(renamed.title.contains("配电网"));
    app.state
        .session_repo
        .rename(&owner.id, s.id, "手工标题", 20)
        .await
        .unwrap();
    message(&app, &owner, s.id, "新的自动标题候选", 21).await;
    let renamed = app
        .state
        .session_repo
        .find(&owner.id, s.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.title, "手工标题");
    assert_eq!(renamed.title_source, "user");
    let foreign = p("foreign");
    let fs = app
        .state
        .session_repo
        .create(&foreign.id, "外部", 1)
        .await
        .unwrap();
    message(&app, &foreign, fs.id, "配电网电压越限", 2).await;
    assert_eq!(
        app.state
            .history_service
            .search_sessions(
                &owner,
                "配电网电压".into(),
                SearchFilters {
                    script_name: None,
                    status: None,
                    from_ms: None,
                    to_ms: None
                },
                20
            )
            .await
            .unwrap()
            .len(),
        1
    );
    app.state
        .session_repo
        .delete(&owner.id, s.id)
        .await
        .unwrap();
    assert!(
        app.state
            .history_service
            .search_sessions(
                &owner,
                "配电网电压".into(),
                SearchFilters {
                    script_name: None,
                    status: None,
                    from_ms: None,
                    to_ms: None
                },
                20
            )
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn durable_operation_card_summaries_survive_fts_rebuild() {
    let app = TestApp::spawn().await;
    let owner = p("instance");
    let session = app
        .state
        .session_repo
        .create(&owner.id, "operation search", 1)
        .await
        .unwrap();
    let card = app
        .state
        .message_repo
        .upsert_operation(
            &owner.id,
            "configuration-1".into(),
            NewMessage {
                session_id: session.id,
                run_id: None,
                role: "tool".into(),
                kind: "study-config".into(),
                content: "研究配置等待确认".into(),
                payload_json: Some("{}".into()),
                created_at_ms: 2,
            },
        )
        .await
        .unwrap();
    app.state.history_service.rebuild_index().await.unwrap();
    let hits = app
        .state
        .history_service
        .search_sessions(
            &owner,
            "配置等待确认".into(),
            SearchFilters {
                script_name: None,
                status: None,
                from_ms: None,
                to_ms: None,
            },
            20,
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    assert_eq!(
        hits[0].message_id.as_deref(),
        Some(card.id.to_string().as_str())
    );
}

#[tokio::test]
async fn messages_support_latest_before_and_around_windows_with_hard_limits() {
    let app = TestApp::spawn().await;
    let owner = p("instance");
    let s = app
        .state
        .session_repo
        .create(&owner.id, "pages", 1)
        .await
        .unwrap();
    let mut ids = Vec::new();
    for i in 0..75 {
        ids.push(message(&app, &owner, s.id, &format!("message {i}"), i).await);
    }
    let latest = app
        .state
        .history_service
        .list_messages(&owner, s.id, None, None, 50)
        .await
        .unwrap();
    assert_eq!(latest.len(), 50);
    assert_eq!(latest[0].id, ids[25]);
    let older = app
        .state
        .history_service
        .list_messages(&owner, s.id, Some(latest[0].id), None, 50)
        .await
        .unwrap();
    assert_eq!(older.len(), 25);
    let around = app
        .state
        .history_service
        .list_messages(&owner, s.id, None, Some(ids[40]), 11)
        .await
        .unwrap();
    assert!(around.iter().any(|m| m.id == ids[40]));
    assert_eq!(around.len(), 11);
    assert_eq!(around[5].id, ids[40]);
    assert!(
        app.state
            .history_service
            .list_messages(&owner, s.id, Some(ids[2]), Some(ids[3]), 10)
            .await
            .unwrap_err()
            .code
            == "history_page_invalid"
    );

    let foreign_session = app
        .state
        .session_repo
        .create(&owner.id, "foreign page", 100)
        .await
        .unwrap();
    let foreign_id = message(&app, &owner, foreign_session.id, "foreign", 101).await;
    for target in [foreign_id, i32::MAX] {
        assert_eq!(
            app.state
                .history_service
                .list_messages(&owner, s.id, None, Some(target), 11)
                .await
                .unwrap_err()
                .code,
            "not_found"
        );
    }
}
