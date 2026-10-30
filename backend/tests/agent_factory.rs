mod common;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use rig::AgentBuilder;
use rig::test_utils::MockCompletionModel;
use rig::tool::server::ToolServer;
use tower::ServiceExt;
use zjudss::agent::{AgentBuildRequest, AgentFactory, LlmAgentFactory, LlmConfig, LlmError};

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
        assert_eq!(*tool_count, zjudss::tools::registered_tool_names().len());
        assert_eq!(*tool_count, 22);
        assert!(prompt.contains("# Skill index"));
        assert!(prompt.contains("model-intake"));
        assert!(prompt.contains("# Engine resources"));
        assert!(prompt.contains("tensoreval-parquet-v3"));
        assert!(!prompt.contains("rss-generation"));
        assert!(!prompt.contains("create_script"));
    }
}

#[test]
fn llm_factory_rejects_unknown_provider() {
    let factory = LlmAgentFactory::new(LlmConfig {
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

struct EnvGuard {
    name: &'static str,
    previous: Option<String>,
}

static ENV_LOCK: Mutex<()> = Mutex::new(());

impl EnvGuard {
    fn set(name: &'static str, value: Option<&str>) -> Self {
        let previous = std::env::var(name).ok();
        // SAFETY: every test that changes these variables holds ENV_LOCK for
        // the complete set/build/restore cycle.
        unsafe {
            match value {
                Some(value) => std::env::set_var(name, value),
                None => std::env::remove_var(name),
            }
        }
        Self { name, previous }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        // SAFETY: see EnvGuard::set; the owner still holds ENV_LOCK on drop.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var(self.name, value),
                None => std::env::remove_var(self.name),
            }
        }
    }
}

fn bailian_factory() -> LlmAgentFactory {
    LlmAgentFactory::new(LlmConfig {
        provider: "bailian".into(),
        model: "qwen3.6-flash".into(),
    })
}

fn build_without_tools(factory: &LlmAgentFactory) -> Result<rig::Agent, LlmError> {
    factory.build(AgentBuildRequest {
        tools: vec![],
        system_prompt: "test".into(),
    })
}

fn expect_build_error(factory: &LlmAgentFactory) -> LlmError {
    match build_without_tools(factory) {
        Ok(_) => panic!("factory build must fail"),
        Err(error) => error,
    }
}

#[test]
fn bailian_factory_accepts_openai_compatible_configuration() {
    let _env_lock = ENV_LOCK.lock().expect("lock Bailian test environment");
    let _key = EnvGuard::set("BAILIAN_API_KEY", Some("bailian-test-secret"));
    let _base_url = EnvGuard::set(
        "BAILIAN_BASE_URL",
        Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
    );

    let result = build_without_tools(&bailian_factory());
    assert!(result.is_ok());
}

#[test]
fn bailian_factory_rejects_missing_or_blank_api_key_without_leaking_it() {
    let _env_lock = ENV_LOCK.lock().expect("lock Bailian test environment");
    let _base_url = EnvGuard::set(
        "BAILIAN_BASE_URL",
        Some("https://dashscope.aliyuncs.com/compatible-mode/v1"),
    );
    for value in [None, Some("   ")] {
        let _key = EnvGuard::set("BAILIAN_API_KEY", value);
        let error = expect_build_error(&bailian_factory());
        assert_eq!(error.to_string(), "缺少或为空的 LLM 配置: BAILIAN_API_KEY");
        assert!(!error.to_string().contains("bailian-test-secret"));
    }
}

#[test]
fn bailian_factory_rejects_missing_or_blank_base_url() {
    let _env_lock = ENV_LOCK.lock().expect("lock Bailian test environment");
    let _key = EnvGuard::set("BAILIAN_API_KEY", Some("bailian-test-secret"));
    for value in [None, Some("   ")] {
        let _base_url = EnvGuard::set("BAILIAN_BASE_URL", value);
        let error = expect_build_error(&bailian_factory());
        assert_eq!(error.to_string(), "缺少或为空的 LLM 配置: BAILIAN_BASE_URL");
        assert!(!error.to_string().contains("bailian-test-secret"));
    }
}

#[test]
fn llm_config_uses_provider_specific_defaults_for_blank_models() {
    let _env_lock = ENV_LOCK.lock().expect("lock LLM test environment");
    for (provider, expected_model) in [
        ("deepseek", rig::providers::deepseek::DEEPSEEK_V4_PRO),
        ("bailian", "qwen3.6-flash"),
    ] {
        let _provider = EnvGuard::set("LLM_PROVIDER", Some(provider));
        let _model = EnvGuard::set("LLM_MODEL", Some("   "));
        let config = LlmConfig::from_env();
        assert_eq!(config.provider, provider);
        assert_eq!(config.model, expected_model);
    }
}
