use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use rig::AgentBuilder;
use rig::agent::{AgentHook, CompletionCallAction, CompletionCallEvent, HookContext};
use rig::test_utils::{MockCompletionModel, MockStreamEvent};
use rig::tool::server::ToolServer;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::Notify;

use crate::agent::{AgentBuildRequest, AgentFactory, LlmError};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ScriptTurn {
    Text {
        text: String,
    },
    ToolCall {
        call_id: String,
        name: String,
        args: Value,
    },
    Error {
        message: String,
    },
    DelayUntilReleased {
        barrier_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScriptScenario {
    pub id: String,
    pub turns: Vec<ScriptTurn>,
    pub expected_prompt: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTraceCall {
    pub turn: usize,
    pub prompt: Value,
    pub history: Value,
    pub scripted_result: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentTrace {
    pub scenario_id: String,
    pub system_prompt: String,
    pub tool_names: Vec<String>,
    pub calls: Vec<AgentTraceCall>,
}

#[derive(Default)]
struct BarrierState {
    entered: AtomicBool,
    released: AtomicBool,
    entered_notify: Notify,
    released_notify: Notify,
}

impl BarrierState {
    async fn enter_and_wait(&self) {
        self.entered.store(true, Ordering::Release);
        self.entered_notify.notify_waiters();
        while !self.released.load(Ordering::Acquire) {
            self.released_notify.notified().await;
        }
    }

    async fn wait_until_entered(&self) {
        while !self.entered.load(Ordering::Acquire) {
            self.entered_notify.notified().await;
        }
    }

    fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.released_notify.notify_waiters();
    }
}

#[derive(Default)]
struct Inner {
    scenarios: Mutex<VecDeque<ScriptScenario>>,
    traces: Mutex<Vec<AgentTrace>>,
    barriers: Mutex<HashMap<String, Arc<BarrierState>>>,
}

#[derive(Clone, Default)]
pub struct ScriptedAgentFactory {
    inner: Arc<Inner>,
}

impl ScriptedAgentFactory {
    pub fn enqueue(&self, scenario: ScriptScenario) {
        {
            let mut barriers = self.inner.barriers.lock().expect("barrier registry");
            for turn in &scenario.turns {
                if let ScriptTurn::DelayUntilReleased { barrier_id } = turn {
                    barriers.entry(barrier_id.clone()).or_default();
                }
            }
        }
        self.inner
            .scenarios
            .lock()
            .expect("scenario queue")
            .push_back(scenario);
    }

    pub fn release(&self, barrier_id: &str) -> bool {
        let barrier = self
            .inner
            .barriers
            .lock()
            .expect("barrier registry")
            .get(barrier_id)
            .cloned();
        if let Some(barrier) = barrier {
            barrier.release();
            true
        } else {
            false
        }
    }

    pub async fn wait_until_blocked(&self, barrier_id: &str) -> bool {
        let barrier = self
            .inner
            .barriers
            .lock()
            .expect("barrier registry")
            .get(barrier_id)
            .cloned();
        if let Some(barrier) = barrier {
            barrier.wait_until_entered().await;
            true
        } else {
            false
        }
    }

    pub fn trace(&self) -> Vec<AgentTrace> {
        self.inner.traces.lock().expect("agent traces").clone()
    }
}

impl AgentFactory for ScriptedAgentFactory {
    fn build(&self, request: AgentBuildRequest) -> Result<rig::Agent, LlmError> {
        let scenario = self
            .inner
            .scenarios
            .lock()
            .expect("scenario queue")
            .pop_front()
            .ok_or_else(|| LlmError::Provider("没有可用的 scripted LLM 场景".into()))?;
        let tool_names = request.tools.iter().map(|tool| tool.name()).collect();
        let trace_index = {
            let mut traces = self.inner.traces.lock().expect("agent traces");
            traces.push(AgentTrace {
                scenario_id: scenario.id.clone(),
                system_prompt: redact_text(request.system_prompt.clone()),
                tool_names,
                calls: Vec::new(),
            });
            traces.len() - 1
        };

        let mut turns = Vec::new();
        let mut results = Vec::new();
        let mut barriers_by_turn = BTreeMap::<usize, Vec<Arc<BarrierState>>>::new();
        let mut model_turn = 1;
        for turn in scenario.turns {
            match turn {
                ScriptTurn::Text { text } => {
                    results.push("text".to_string());
                    turns.push(vec![
                        MockStreamEvent::text(text),
                        MockStreamEvent::final_response_with_default_usage(),
                    ]);
                    model_turn += 1;
                }
                ScriptTurn::ToolCall {
                    call_id,
                    name,
                    args,
                } => {
                    results.push(format!("tool:{name}"));
                    turns.push(vec![
                        MockStreamEvent::tool_call(&call_id, name, args).with_call_id(call_id),
                        MockStreamEvent::final_response_with_default_usage(),
                    ]);
                    model_turn += 1;
                }
                ScriptTurn::Error { message } => {
                    results.push("error".to_string());
                    turns.push(vec![MockStreamEvent::error(message)]);
                    model_turn += 1;
                }
                ScriptTurn::DelayUntilReleased { barrier_id } => {
                    let barrier = self
                        .inner
                        .barriers
                        .lock()
                        .expect("barrier registry")
                        .entry(barrier_id)
                        .or_default()
                        .clone();
                    barriers_by_turn
                        .entry(model_turn)
                        .or_default()
                        .push(barrier);
                }
            }
        }
        if turns.is_empty() {
            return Err(LlmError::Provider(format!(
                "scripted LLM 场景 {} 没有响应 turn",
                scenario.id
            )));
        }

        let hook = ScriptHook {
            expected_prompt: scenario.expected_prompt,
            barriers_by_turn,
            results,
            inner: self.inner.clone(),
            trace_index,
        };
        let mut server = ToolServer::new();
        for tool in request.tools {
            server = server.registered_tool(tool);
        }
        Ok(
            AgentBuilder::new(MockCompletionModel::from_stream_turns(turns))
                .preamble(request.system_prompt)
                .add_hook(hook)
                .tool_server_handle(server.run())
                .build(),
        )
    }
}

#[derive(Clone)]
struct ScriptHook {
    expected_prompt: Option<String>,
    barriers_by_turn: BTreeMap<usize, Vec<Arc<BarrierState>>>,
    results: Vec<String>,
    inner: Arc<Inner>,
    trace_index: usize,
}

impl AgentHook for ScriptHook {
    async fn on_completion_call(
        &self,
        _ctx: &HookContext,
        event: CompletionCallEvent<'_>,
    ) -> CompletionCallAction {
        let prompt = redact_value(serde_json::to_value(event.prompt).unwrap_or(Value::Null));
        if event.turn == 1
            && let Some(expected) = &self.expected_prompt
            && !prompt.to_string().contains(expected)
        {
            return CompletionCallAction::stop(format!(
                "scripted prompt 不匹配，期望包含 {expected}"
            ));
        }
        if let Some(barriers) = self.barriers_by_turn.get(&event.turn) {
            for barrier in barriers {
                barrier.enter_and_wait().await;
            }
        }
        let history = redact_value(serde_json::to_value(event.history).unwrap_or(Value::Null));
        if let Some(trace) = self
            .inner
            .traces
            .lock()
            .expect("agent traces")
            .get_mut(self.trace_index)
        {
            trace.calls.push(AgentTraceCall {
                turn: event.turn,
                prompt,
                history,
                scripted_result: self
                    .results
                    .get(event.turn.saturating_sub(1))
                    .cloned()
                    .unwrap_or_else(|| "unscripted".into()),
            });
        }
        CompletionCallAction::continue_run()
    }
}

fn redact_text(value: String) -> String {
    if value.len() > 256 * 1024 {
        "<redacted:oversize>".into()
    } else {
        value
    }
}

fn redact_value(value: Value) -> Value {
    match value {
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| {
                    let lower = key.to_ascii_lowercase();
                    if [
                        "authorization",
                        "api_key",
                        "apikey",
                        "secret",
                        "token",
                        "file_bytes",
                    ]
                    .iter()
                    .any(|sensitive| lower.contains(sensitive))
                    {
                        (key, Value::String("<redacted>".into()))
                    } else {
                        (key, redact_value(value))
                    }
                })
                .collect(),
        ),
        Value::Array(values) => Value::Array(values.into_iter().map(redact_value).collect()),
        Value::String(value) if value.len() > 64 * 1024 => {
            Value::String("<redacted:oversize>".into())
        }
        other => other,
    }
}
