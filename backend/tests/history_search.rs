mod common;
use backend::{
    application::now_ms,
    auth::{AccessScope, Principal, StorageScopeRef},
    db::{
        models::{NewMessage, NewSimulationTask},
        repositories::{SearchFilters, TaskRepo},
    },
};
use common::TestApp;
use diesel::RunQueryDsl;
fn p(key: &str) -> Principal {
    Principal {
        access: AccessScope {
            kind: "instance".into(),
            key: key.into(),
        },
        storage: StorageScopeRef {
            kind: "instance".into(),
            key: key.into(),
        },
        subject: key.into(),
    }
}
async fn message(app: &TestApp, p: &Principal, session: i32, text: &str, at: i64) -> i32 {
    app.state
        .message_repo
        .create(
            &p.access,
            NewMessage {
                session_id: session,
                task_id: None,
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
async fn chinese_substring_short_query_filters_delete_and_scope_are_bounded() {
    let app = TestApp::spawn().await;
    let owner = p("instance");
    let s = app
        .state
        .session_repo
        .create(&owner.access, "", 1)
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
                study_type: None,
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
                study_type: None,
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
        .find(&owner.access, s.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.title_source, "auto");
    assert!(renamed.title.contains("配电网"));
    app.state
        .session_repo
        .rename(&owner.access, s.id, "手工标题", 20)
        .await
        .unwrap();
    message(&app, &owner, s.id, "新的自动标题候选", 21).await;
    let renamed = app
        .state
        .session_repo
        .find(&owner.access, s.id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(renamed.title, "手工标题");
    assert_eq!(renamed.title_source, "user");
    let foreign = p("foreign");
    let fs = app
        .state
        .session_repo
        .create(&foreign.access, "外部", 1)
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
                    study_type: None,
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
        .delete(&owner.access, s.id)
        .await
        .unwrap();
    assert!(
        app.state
            .history_service
            .search_sessions(
                &owner,
                "配电网电压".into(),
                SearchFilters {
                    study_type: None,
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
async fn messages_support_latest_before_and_around_windows_with_hard_limits() {
    let app = TestApp::spawn().await;
    let owner = p("instance");
    let s = app
        .state
        .session_repo
        .create(&owner.access, "pages", 1)
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
    assert!(
        app.state
            .history_service
            .list_messages(&owner, s.id, Some(ids[2]), Some(ids[3]), 10)
            .await
            .unwrap_err()
            .code
            == "history_page_invalid"
    );
}

#[tokio::test]
async fn task_filters_work_and_missing_fts_does_not_break_plain_history_reads() {
    let app = TestApp::spawn().await;
    let owner = p("instance");
    let s = app
        .state
        .session_repo
        .create(&owner.access, "tasks", 1)
        .await
        .unwrap();
    TaskRepo::new(app.pool.clone())
        .create(
            &owner.access,
            NewSimulationTask {
                session_id: s.id,
                status: "succeeded".into(),
                stage: Some("done".into()),
                study_type: Some("power_flow".into()),
                params_json: "{}".into(),
                exit_code: Some(0),
                error_json: None,
                created_at_ms: now_ms(),
                started_at_ms: None,
                finished_at_ms: None,
            },
        )
        .await
        .unwrap();
    let hits = app
        .state
        .history_service
        .search_sessions(
            &owner,
            "power".into(),
            SearchFilters {
                study_type: Some("power_flow".into()),
                status: Some("succeeded".into()),
                from_ms: None,
                to_ms: None,
            },
            20,
        )
        .await
        .unwrap();
    assert_eq!(hits.len(), 1);
    let mut conn = app.pool.get().unwrap();
    diesel::sql_query("DROP TABLE chat_search_fts")
        .execute(&mut conn)
        .unwrap();
    drop(conn);
    let e = app
        .state
        .history_service
        .search_sessions(
            &owner,
            "power".into(),
            SearchFilters {
                study_type: None,
                status: None,
                from_ms: None,
                to_ms: None,
            },
            20,
        )
        .await
        .unwrap_err();
    assert_eq!(e.code, "history_search_unavailable");
    assert!(
        app.state
            .history_service
            .list_messages(&owner, s.id, None, None, 50)
            .await
            .is_ok()
    );
}
