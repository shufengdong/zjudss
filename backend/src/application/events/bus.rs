//! session 级实时事件广播通道。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use tokio::sync::broadcast;

use super::AppEvent;

/// 按 session 隔离的事件总线（每条 session 一条 broadcast 通道）。
#[derive(Clone, Default)]
pub struct EventBus {
    senders: Arc<Mutex<HashMap<i32, broadcast::Sender<AppEvent>>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self::default()
    }

    /// 订阅某 session 的事件流（懒创建通道）。
    pub fn subscribe(&self, session_id: i32) -> broadcast::Receiver<AppEvent> {
        let mut senders = self.senders.lock().unwrap();
        senders
            .entry(session_id)
            .or_insert_with(|| broadcast::channel(256).0)
            .subscribe()
    }

    /// 发布一个事件；无订阅者时丢弃。
    pub fn publish(&self, session_id: i32, event: AppEvent) {
        let senders = self.senders.lock().unwrap();
        if let Some(tx) = senders.get(&session_id) {
            let _ = tx.send(event);
        }
    }

    /// 注销某 session 的通道（删除会话时调用）；drop 最后 sender 使开着的 SSE 流收尾，
    /// 避免 `senders` 映射随会话数无限增长。
    pub fn unregister(&self, session_id: i32) {
        self.senders.lock().unwrap().remove(&session_id);
    }
}
