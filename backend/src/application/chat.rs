//! ChatService：一次对话的编排。
//!
//! 每次 `run` 构建一套有状态工具 + agent，用 rig 的 `run_channel()` 拆出
//! 「驱动 future」与「事件 feed」，把 `MultiTurnStreamItem` 映射为 `AppEvent`
//! （architecture §4），逐条落库 + 发布到 session 的 SSE 通道。

use std::collections::BTreeSet;
use std::sync::Arc;

use futures::StreamExt;
use rig::agent::{MultiTurnStreamItem, RunEvents};
use rig::message::{AssistantContent, Message, ToolResultContent};
use rig::streaming::{Item, StreamEvent};

use crate::adapters::SimEngine;
use crate::agent::{self, LlmConfig};
use crate::application::{AppState, EventBus, EventWriter, ToolStatus, now_ms};
use crate::auth::Principal;
use crate::db::repositories::{MessageRepo, SessionRepo, TaskRepo};
use crate::tools::{self, ToolServices};
use tokio::sync::Semaphore;

/// 单次对话允许的最大模型调用轮数（防死循环）。
const MAX_TURNS: usize = 8;

#[derive(Clone)]
pub struct ChatService {
    engine: Arc<dyn SimEngine>,
    events: EventBus,
    message_repo: MessageRepo,
    session_repo: SessionRepo,
    task_repo: TaskRepo,
    llm: LlmConfig,
    semaphore: Arc<Semaphore>,
    principal: Principal,
    workspaces: crate::application::WorkspaceService,
    example_import: crate::application::ExampleImportService,
    model_patch: crate::application::ModelPatchService,
    engine_snapshot: Arc<crate::engine_resources::EngineResourceSnapshot>,
    agent_knowledge: Arc<crate::agent::knowledge::AgentKnowledge>,
    simulations: crate::application::SimulationService,
    datasets: crate::application::DatasetService,
    analysis: crate::application::AnalysisService,
    comparisons: crate::application::ComparisonService,
    history: crate::application::HistoryService,
}

impl ChatService {
    pub fn new(state: &AppState, principal: Principal) -> Self {
        Self {
            engine: state.engine.clone(),
            events: state.events.clone(),
            message_repo: state.message_repo.clone(),
            session_repo: state.session_repo.clone(),
            task_repo: state.task_repo.clone(),
            llm: state.llm.clone(),
            semaphore: state.semaphore.clone(),
            principal,
            workspaces: state.workspace_service.clone(),
            example_import: state.example_import_service.clone(),
            model_patch: state.model_patch_service.clone(),
            engine_snapshot: state.engine_snapshot.clone(),
            agent_knowledge: state.agent_knowledge.clone(),
            simulations: state.simulation_service.clone(),
            datasets: state.dataset_service.clone(),
            analysis: state.analysis_service.clone(),
            comparisons: state.comparison_service.clone(),
            history: state.history_service.clone(),
        }
    }

    /// 触发一次对话。`prompt_id` 是刚落库的 user 消息 id，历史只取比它更早的消息。
    pub async fn run(&self, session_id: i32, prompt_id: i32, prompt: String) {
        let writer = EventWriter::new(
            self.principal.access.clone(),
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
            task_repo: self.task_repo.clone(),
            semaphore: self.semaphore.clone(),
            scope: self.principal.access.clone(),
            principal: self.principal.clone(),
            workspaces: self.workspaces.clone(),
            example_import: self.example_import.clone(),
            model_patch: self.model_patch.clone(),
            engine_snapshot: self.engine_snapshot.clone(),
            agent_knowledge: self.agent_knowledge.clone(),
            loaded_skills: Arc::new(tokio::sync::Mutex::new(BTreeSet::new())),
            simulations: self.simulations.clone(),
            datasets: self.datasets.clone(),
            analysis: self.analysis.clone(),
            comparisons: self.comparisons.clone(),
            history: self.history.clone(),
        });
        let tools = tools::build_tools(services);

        // 2. 构建 agent。
        let system_prompt = crate::agent::prompt::PromptAssembler::assemble(
            &self.agent_knowledge,
            &self.engine_snapshot,
        );
        let agent = match agent::build_agent_with_prompt(&self.llm, tools, system_prompt) {
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

        let handle = tokio::spawn(future);

        // 5. 消费事件流 → 映射 → 落库 + 发布。
        self.consume(events, &writer).await;

        // 6. 等 agent future 收尾；异常时补一条 assistant 错误。
        match handle.await {
            Ok(Ok(_response)) => { /* 最终文本已在 End 段发过 */ }
            Ok(Err(err)) => {
                let _ = writer.assistant(&format!("agent 运行失败：{err}")).await;
            }
            Err(join) => {
                let _ = writer.assistant(&format!("agent 任务异常：{join}")).await;
            }
        }
        let _ = self
            .session_repo
            .touch(&self.principal.access, session_id, now_ms())
            .await;
    }

    /// 加载本次 prompt 之前的历史 user/assistant 消息，转成 rig `Message`。
    async fn load_history(&self, session_id: i32, prompt_id: i32) -> Vec<Message> {
        let msgs = self
            .message_repo
            .list_by_session(&self.principal.access, session_id)
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
    async fn consume(&self, mut events: RunEvents, writer: &EventWriter) {
        // 按 call id 记住 args，让 success/failed 事件复用 running 事件的参数。
        let mut args_by_call: std::collections::HashMap<String, serde_json::Value> =
            std::collections::HashMap::new();

        while let Some(item) = events.next().await {
            match item {
                // 文本段结束 → assistant（§4：StreamAssistantItem → assistant）。
                MultiTurnStreamItem::StreamAssistantItem(Item::Event(StreamEvent::End {
                    content,
                    ..
                })) => {
                    if let AssistantContent::Text(text) = content {
                        if !text.text.trim().is_empty() {
                            let _ = writer.assistant(&text.text).await;
                        }
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
