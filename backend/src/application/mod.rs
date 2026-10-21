//! 编排层：全局状态、事件总线与一次对话的编排。
//!
//! `AppState` 是 axum 的共享状态（连接池、仓储、事件总线、引擎适配器、限流信号量）；
//! `EventBus` 按 session 广播事件供 SSE 订阅；`ChatService` 编排一次 agent 对话。

mod events;
pub mod chat;

use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::Semaphore;

use crate::adapters::{resolve_data_dir, SimEngine};
use crate::agent::LlmConfig;
use crate::db::repo::{MessageRepo, SessionRepo, TaskRepo};
use crate::db::DbPool;

pub use chat::ChatService;
pub use events::{
    AppEvent, AttachmentFile, EventBus, EventWriter, Metrics, Stage, StageStatus, ToolStatus,
    ms_to_iso, now_ms,
};

/// 全局共享状态（axum `State`）。
#[derive(Clone)]
pub struct AppState {
    pub pool: DbPool,
    pub session_repo: SessionRepo,
    pub message_repo: MessageRepo,
    pub task_repo: TaskRepo,
    pub events: EventBus,
    pub engine: Arc<dyn SimEngine>,
    pub llm: LlmConfig,
    pub semaphore: Arc<Semaphore>,
    /// 数据根目录（内核结果、上传附件落这里），由 `DATA_DIR` 环境变量或默认 `data` 决定。
    pub data_dir: PathBuf,
}

impl AppState {
    pub fn new(pool: DbPool, engine: Arc<dyn SimEngine>, llm: LlmConfig) -> Self {
        let session_repo = SessionRepo::new(pool.clone());
        let message_repo = MessageRepo::new(pool.clone());
        let task_repo = TaskRepo::new(pool.clone());
        Self {
            pool,
            session_repo,
            message_repo,
            task_repo,
            events: EventBus::new(),
            engine,
            llm,
            semaphore: Arc::new(Semaphore::new(4)),
            data_dir: resolve_data_dir(),
        }
    }

    /// 派生一次对话的编排服务（共享引擎/事件/仓储，克隆成本低）。
    pub fn chat(&self) -> ChatService {
        ChatService::new(self)
    }
}
