mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use backend::agent::{AgentBuildRequest, AgentFactory, DeepSeekAgentFactory, LlmConfig, LlmError};
use rig::AgentBuilder;
use rig::test_utils::MockCompletionModel;
use rig::tool::server::ToolServer;
use tower::ServiceExt;

use common::TestApp;

#[derive(Default)]
struct RecordingFactory {
    builds: Mutex<Vec<(usize, String)>>,
}

impl AgentFactory for RecordingFactory {
    fn build(&self, request: AgentBuildRequest) -> Result<rig::Agent, LlmError> {
        self.builds
            .lock()
            .expect("record builds")
            .push((request.tools.len(), request.system_prompt.clone()));
        let mut server = ToolServer::new();
        for tool in request.tools {
            server = server.registered_tool(tool);
        }
        Ok(AgentBuilder::new(MockCompletionModel::text("ok"))
            .preamble(request.system_prompt)
            .tool_server_handle(server.run())
            .build())
    }
}

#[tokio::test]
async fn chat_builds_one_agent_per_user_message() {
    let factory = Arc::new(RecordingFactory::default());
    let app = TestApp::spawn_with_agent_factory(factory.clone()).await;
    let cookie = app.login_cookie("instance").await;
    let create = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/sessions")
                .header("content-type", "application/json")
                .header("cookie", &cookie)
                .body(Body::from(r#"{"title":"factory"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(create.status(), StatusCode::CREATED);
    let body = axum::body::to_bytes(create.into_body(), 1024)
        .await
        .unwrap();
    let session_id = serde_json::from_slice::<serde_json::Value>(&body).unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();

    for message in ["第一次", "第二次"] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/sessions/{session_id}/messages"))
                    .header("content-type", "application/json")
                    .header("cookie", &cookie)
                    .body(Body::from(
                        serde_json::json!({"message": message}).to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        if status != StatusCode::ACCEPTED {
            let body = axum::body::to_bytes(response.into_body(), 1024)
                .await
                .unwrap();
            panic!(
                "posting {message:?} returned {status}: {}",
                String::from_utf8_lossy(&body)
            );
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while app.state.scheduler.active_count() != 0
                || app.state.scheduler.waiting_count() != 0
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("agent run becomes idle");
    }

    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if factory.builds.lock().unwrap().len() == 2 {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("both agent builds complete");

    let builds = factory.builds.lock().unwrap();
    assert_eq!(builds.len(), 2);
    for (tool_count, prompt) in builds.iter() {
        assert_eq!(*tool_count, backend::tools::registered_tool_names().len());
        assert!(prompt.contains("# Skill index"));
        assert!(prompt.contains("model-intake"));
        assert!(prompt.contains("# Engine resources"));
        assert!(prompt.contains("tensoreval-parquet-v3"));
    }
}

#[test]
fn deepseek_factory_rejects_unknown_provider() {
    let factory = DeepSeekAgentFactory::new(LlmConfig {
        provider: "unknown".into(),
        model: "irrelevant".into(),
    });
    let error = match factory.build(AgentBuildRequest {
        tools: vec![],
        system_prompt: "test".into(),
    }) {
        Ok(_) => panic!("unknown provider must fail"),
        Err(error) => error,
    };
    assert!(matches!(error, LlmError::UnsupportedProvider(ref provider) if provider == "unknown"));
    assert_eq!(error.to_string(), "不支持的 provider: unknown");
}
