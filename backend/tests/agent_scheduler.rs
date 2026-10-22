mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use backend::application::{RunScheduler, SchedulerError};
use backend::auth::{AccessScope, AuthConfig};
use serde_json::{Value, json};
use tokio::task::JoinSet;
use tower::ServiceExt;

use common::TestApp;

fn scope(key: &str) -> AccessScope {
    AccessScope {
        kind: "owner".to_string(),
        key: key.to_string(),
    }
}

async fn observed_max(
    scheduler: RunScheduler,
    jobs: impl IntoIterator<Item = (AccessScope, i32)>,
) -> usize {
    let active = Arc::new(AtomicUsize::new(0));
    let maximum = Arc::new(AtomicUsize::new(0));
    let mut set = JoinSet::new();
    for (owner, session_id) in jobs {
        let ticket = scheduler
            .try_admit(&owner, session_id)
            .expect("job should fit waiting queue");
        let active = active.clone();
        let maximum = maximum.clone();
        set.spawn(async move {
            ticket
                .run(async move {
                    let now = active.fetch_add(1, Ordering::SeqCst) + 1;
                    maximum.fetch_max(now, Ordering::SeqCst);
                    tokio::time::sleep(Duration::from_millis(20)).await;
                    active.fetch_sub(1, Ordering::SeqCst);
                })
                .await;
        });
    }
    while set.join_next().await.is_some() {}
    maximum.load(Ordering::SeqCst)
}

#[tokio::test]
async fn enforces_global_owner_and_session_limits() {
    let scheduler = RunScheduler::new(8, 2, 32);
    let global_jobs = (0..16).map(|index| (scope(&format!("owner-{index}")), index));
    assert_eq!(observed_max(scheduler, global_jobs).await, 8);

    let scheduler = RunScheduler::new(8, 2, 32);
    let owner_jobs = (0..8).map(|index| (scope("alice"), index));
    assert_eq!(observed_max(scheduler, owner_jobs).await, 2);

    let scheduler = RunScheduler::new(8, 2, 32);
    let session_jobs = (0..4).map(|_| (scope("alice"), 7));
    assert_eq!(observed_max(scheduler, session_jobs).await, 1);
}

#[test]
fn rejects_the_thirty_third_waiting_run() {
    let scheduler = RunScheduler::new(8, 2, 32);
    let active: Vec<_> = (0..8)
        .map(|index| {
            scheduler
                .try_admit(&scope(&format!("active-{index}")), index)
                .expect("active run")
        })
        .collect();
    assert_eq!(scheduler.active_count(), 8);

    let queued: Vec<_> = (0..32)
        .map(|index| {
            scheduler
                .try_admit(&scope(&format!("queued-{index}")), 100 + index)
                .expect("queued run")
        })
        .collect();
    assert_eq!(scheduler.waiting_count(), 32);
    assert_eq!(
        scheduler.try_admit(&scope("overflow"), 999).unwrap_err(),
        SchedulerError::QueueFull
    );

    drop((active, queued));
    assert_eq!(scheduler.active_count(), 0);
    assert_eq!(scheduler.waiting_count(), 0);
}

async fn response_json(response: axum::response::Response) -> Value {
    let bytes = to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("read response body");
    serde_json::from_slice(&bytes).expect("parse response json")
}

#[tokio::test]
async fn queue_rejection_has_stable_error_and_does_not_write_message() {
    let scheduler = RunScheduler::new(1, 1, 1);
    let app = TestApp::spawn_with_scheduler(AuthConfig::instance(), scheduler.clone()).await;

    let create = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"title":"queue test"}"#))
                .expect("create request"),
        )
        .await
        .expect("create session");
    let session_id = response_json(create).await["id"]
        .as_str()
        .expect("session id")
        .to_string();

    let instance = AccessScope {
        kind: "instance".to_string(),
        key: "instance".to_string(),
    };
    let _active = scheduler.try_admit(&instance, 900).expect("active slot");
    let _queued = scheduler.try_admit(&instance, 901).expect("waiting slot");

    let rejected = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/api/sessions/{session_id}/messages"))
                .header("content-type", "application/json")
                .body(Body::from(r#"{"message":"should not persist"}"#))
                .expect("post message request"),
        )
        .await
        .expect("post message response");
    assert_eq!(rejected.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(
        response_json(rejected).await,
        json!({
            "error": "Agent 请求队列已满，请稍后重试",
            "code": "agent_queue_full"
        })
    );

    let history = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .uri(format!("/api/sessions/{session_id}/messages"))
                .body(Body::empty())
                .expect("history request"),
        )
        .await
        .expect("history response");
    assert_eq!(response_json(history).await["messages"], json!([]));
}
