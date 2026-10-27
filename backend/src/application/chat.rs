//! ChatService：一次对话的编排。
//!
//! 每次 `run` 构建一套有状态工具 + agent，用 rig 的 `run_channel()` 拆出
//! 「驱动 future」与「事件 feed」，把 `MultiTurnStreamItem` 映射为 `AppEvent`
//! （architecture §4），逐条落库 + 发布到 session 的 SSE 通道。

use std::collections::BTreeSet;
use std::sync::Arc;

use futures::StreamExt;
use rig::agent::{MultiTurnStreamItem, RunEvents};
use rig::completion::PromptError;
use rig::message::{AssistantContent, Message, ToolResultContent};
use rig::streaming::{Item, StreamEvent};

use crate::adapters::SimEngine;
use crate::agent::{AgentBuildRequest, AgentFactory};
use crate::application::{AppState, EventBus, EventWriter, ToolStatus, now_ms};
use crate::auth::CurrentUser;
use crate::db::repositories::{MessageRepo, SessionRepo};
use crate::tools::{self, ToolServices};
use tokio::sync::Semaphore;

/// 单次对话允许的最大模型调用轮数（防死循环）。
const MAX_TURNS: usize = 8;
const CONFIG_PREPARED_REPLY: &str = "研究配置已准备，请在卡片中补齐并确认输入、参数和结果";
const TURN_LIMIT_REPLY: &str = "本轮工具调用已达到上限，尚未完成研究配置，请重新发送请求";
const AGENT_FAILED_REPLY: &str = "agent 运行失败，请稍后重试";

#[derive(Default)]
struct TurnOutcome {
    assistant_emitted: bool,
    configuration_prepared: bool,
}

#[derive(Clone)]
pub struct ChatService {
    engine: Arc<dyn SimEngine>,
    events: EventBus,
    message_repo: MessageRepo,
    session_repo: SessionRepo,
    agent_factory: Arc<dyn AgentFactory>,
    semaphore: Arc<Semaphore>,
    principal: CurrentUser,
    model_patch: crate::application::ModelPatchService,
    model_dataframes: crate::application::ModelDataFrameReadService,
    scripts: crate::application::ScriptService,
    versions: crate::application::VersionService,
    runs: crate::application::RunService,
    engine_snapshot: Arc<crate::engine_resources::EngineResourceSnapshot>,
    agent_knowledge: Arc<crate::agent::knowledge::AgentKnowledge>,
    datasets: crate::application::DatasetService,
    analysis: crate::application::AnalysisService,
    comparisons: crate::application::ComparisonService,
    history: crate::application::HistoryService,
    artifact_service: crate::application::ArtifactService,
    study_configurations: crate::application::StudyConfigurationService,
    data_resources: crate::application::DataResourceService,
    session_cancellations: crate::application::SessionCancellationRegistry,
}

impl ChatService {
    pub fn new(state: &AppState, principal: CurrentUser) -> Self {
        Self {
            engine: state.engine.clone(),
            events: state.events.clone(),
            message_repo: state.message_repo.clone(),
            session_repo: state.session_repo.clone(),
            agent_factory: state.agent_factory.clone(),
            semaphore: state.semaphore.clone(),
            principal,
            model_patch: state.model_patch_service.clone(),
            model_dataframes: state.model_dataframe_read_service.clone(),
            scripts: state.script_service.clone(),
            versions: state.version_service.clone(),
            runs: state.run_service.clone(),
            engine_snapshot: state.engine_snapshot.clone(),
            agent_knowledge: state.agent_knowledge.clone(),
            datasets: state.dataset_service.clone(),
            analysis: state.analysis_service.clone(),
            comparisons: state.comparison_service.clone(),
            history: state.history_service.clone(),
            artifact_service: state.artifact_service.clone(),
            study_configurations: state.study_configuration_service.clone(),
            data_resources: state.data_resource_service.clone(),
            session_cancellations: state.session_cancellations.clone(),
        }
    }

    /// 触发一次对话。`prompt_id` 是刚落库的 user 消息 id，历史只取比它更早的消息。
    pub async fn run(&self, session_id: i32, prompt_id: i32, prompt: String) {
        let mut cancelled = self
            .session_cancellations
            .subscribe(&self.principal.id, session_id);
        if *cancelled.borrow() {
            return;
        }
        let writer = EventWriter::new(
            self.principal.id.clone(),
            session_id,
            self.message_repo.clone(),
            self.events.clone(),
        );

        // 1. 每请求构建一套有状态工具（§5 / §10 #2）。
        let services = Arc::new(ToolServices {
            engine: self.engine.clone(),
            writer: writer.clone(),
            session_id,
            request_message_id: prompt_id,
            semaphore: self.semaphore.clone(),
            scope: self.principal.id.clone(),
            principal: self.principal.clone(),
            model_patch: self.model_patch.clone(),
            model_dataframes: self.model_dataframes.clone(),
            scripts: self.scripts.clone(),
            versions: self.versions.clone(),
            runs: self.runs.clone(),
            engine_snapshot: self.engine_snapshot.clone(),
            agent_knowledge: self.agent_knowledge.clone(),
            loaded_skills: Arc::new(tokio::sync::Mutex::new(BTreeSet::new())),
            study_configurations: self.study_configurations.clone(),
            datasets: self.datasets.clone(),
            analysis: self.analysis.clone(),
            comparisons: self.comparisons.clone(),
            history: self.history.clone(),
            artifact_service: self.artifact_service.clone(),
            data_resources: self.data_resources.clone(),
        });
        let tools = tools::build_tools(services);

        // 2. 构建 agent。
        let system_prompt = crate::agent::prompt::PromptAssembler::assemble(
            &self.agent_knowledge,
            &self.engine_snapshot,
        );
        let agent = match self.agent_factory.build(AgentBuildRequest {
            tools,
            system_prompt,
        }) {
            Ok(agent) => agent,
            Err(err) => {
                let _ = writer.assistant(&format!("无法启动 agent：{err}")).await;
                return;
            }
        };

        // 3. 历史（本次 prompt 之前）。
        let history = self.load_history(session_id, prompt_id).await;

        // 4. run_channel：拆出驱动 future 与事件 feed（§4）。
        let (future, events) = agent
            .prompt(prompt)
            .history(history)
            .max_turns(MAX_TURNS)
            .run_channel();

        let mut handle = tokio::spawn(future);

        // 5. 消费事件流 → 映射 → 落库 + 发布。
        let outcome = tokio::select! {
            _ = cancelled.changed() => {
                handle.abort();
                return;
            }
            outcome = self.consume(events, &writer) => outcome,
        };

        // 6. 等 agent future 收尾。prepare_study 已成功即完成本轮领域操作，
        // 后续生成收尾文本的 provider 错误不得覆盖已持久化的成功结果。
        let agent_result = tokio::select! {
            _ = cancelled.changed() => {
                handle.abort();
                return;
            }
            result = &mut handle => result,
        };
        let failure_reply = match agent_result {
            Ok(Ok(_response)) => None,
            Ok(Err(PromptError::MaxTurns { max_turns, .. })) => {
                eprintln!(
                    "agent turn limit reached: session_id={session_id}, max_turns={max_turns}"
                );
                Some(TURN_LIMIT_REPLY)
            }
            Ok(Err(error)) => {
                eprintln!("agent run failed: session_id={session_id}, error={error}");
                Some(AGENT_FAILED_REPLY)
            }
            Err(error) => {
                eprintln!("agent task failed: session_id={session_id}, error={error}");
                Some(AGENT_FAILED_REPLY)
            }
        };
        if outcome.configuration_prepared {
            if !outcome.assistant_emitted {
                let _ = writer.assistant(CONFIG_PREPARED_REPLY).await;
            }
        } else if let Some(reply) = failure_reply {
            let _ = writer.assistant(reply).await;
        }
        let _ = self
            .session_repo
            .touch(&self.principal.id, session_id, now_ms())
            .await;
    }

    /// 加载本次 prompt 之前的历史 user/assistant 消息，转成 rig `Message`。
    async fn load_history(&self, session_id: i32, prompt_id: i32) -> Vec<Message> {
        let msgs = self
            .message_repo
            .list_by_session(&self.principal.id, session_id)
            .await
            .unwrap_or_default();
        msgs.into_iter()
            .filter(|m| m.id < prompt_id)
            .filter(|m| m.role == "user" || m.role == "assistant")
            .map(|m| match m.role.as_str() {
                "user" => Message::user(m.content),
                _ => Message::assistant(m.content),
            })
            .collect()
    }

    /// 把 `RunEvents` 逐个映射为 `AppEvent` 并落库 + 发布。
    async fn consume(&self, mut events: RunEvents, writer: &EventWriter) -> TurnOutcome {
        // 按 call id 记住 args，让 success/failed 事件复用 running 事件的参数。
        let mut args_by_call: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();
        let mut outcome = TurnOutcome::default();

        while let Some(item) = events.next().await {
            match item {
                // 文本段结束 → assistant（§4：StreamAssistantItem → assistant）。
                MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::End {
                    content: AssistantContent::Text(text),
                    ..
                })) if !text.text.trim().is_empty() => {
                    if writer.assistant(&text.text).await.is_ok() {
                        outcome.assistant_emitted = true;
                    }
                }
                // 工具调用已提交 → tool-call(running)（§4）。
                MultiTurnStreamItem::ToolCall { tool_call } => {
                    let args = tool_call.function.arguments_value();
                    args_by_call.insert(tool_call.id.to_string(), args.clone());
                    let _ = writer
                        .tool_call(
                            &tool_call.id.to_string(),
                            tool_call.function.name.as_str(),
                            args,
                            ToolStatus::Running,
                            None,
                        )
                        .await;
                }
                // 工具结果 → tool-call(success/failed)（§4）。
                MultiTurnStreamItem::ToolResult { tool_result } => {
                    let status = if tool_result.is_error {
                        ToolStatus::Failed
                    } else {
                        ToolStatus::Success
                    };
                    if !tool_result.is_error && tool_result.name.as_str() == "prepare_study" {
                        outcome.configuration_prepared = true;
                    }
                    let summary = summarize(&tool_result.content);
                    let args = args_by_call
                        .get(&tool_result.call.to_string())
                        .cloned()
                        .unwrap_or(serde_json::Value::Null);
                    let _ = writer
                        .tool_call(
                            &tool_result.call.to_string(),
                            tool_result.name.as_str(),
                            args,
                            status,
                            Some(summary),
                        )
                        .await;
                }
                // FinalResponse / 其它 → 忽略（文本已在 End 段发过）。
                _ => {}
            }
        }
        outcome
    }
}

/// 从工具结果内容提取可读摘要（文本拼接，JSON 序列化）。
fn summarize(content: &[ToolResultContent]) -> String {
    let parts: Vec<String> = content
        .iter()
        .map(|c| match c {
            ToolResultContent::Text(t) => t.text.clone(),
            ToolResultContent::Json { value } => value.to_string(),
            ToolResultContent::Image(_) => "<image>".to_string(),
        })
        .collect();
    let s = parts.join("\n");
    if s.trim().is_empty() {
        "(无输出)".to_string()
    } else {
        s
    }
}
