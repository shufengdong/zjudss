//! 事件总线与推给前端的结构化消息（对齐 `frontend/src/types/index.ts`）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::auth::AccessScope;
use crate::db::models::{Message, NewMessage};
use crate::db::repositories::{MessageRepo, RepoError};

/// 毫秒时间戳 → ISO 8601 字符串（前端 `timestamp` 字段用）。
pub fn ms_to_iso(ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ms)
        .map(|dt| dt.to_rfc3339())
        .unwrap_or_default()
}

/// 当前毫秒时间戳。
pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---- 前端消息的子结构 ----

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolStatus {
    Running,
    Success,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StageStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Stage {
    pub key: String,
    pub label: String,
    pub status: StageStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metrics {
    pub max_voltage: f64,
    pub min_voltage: f64,
    pub violation_nodes: i64,
    pub violation_duration: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentFile {
    pub file_name: String,
    pub file_size: String,
    pub file_type: String,
}

/// 推给前端的结构化消息（discriminated union，`type` 区分）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppEvent {
    User {
        id: String,
        timestamp: String,
        content: String,
    },
    Assistant {
        id: String,
        timestamp: String,
        content: String,
    },
    #[serde(rename = "tool-call", rename_all = "camelCase")]
    ToolCall {
        id: String,
        timestamp: String,
        tool_name: String,
        args: serde_json::Value,
        status: ToolStatus,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Progress {
        id: String,
        timestamp: String,
        task_id: String,
        stages: Vec<Stage>,
    },
    #[serde(rename_all = "camelCase")]
    Result {
        id: String,
        timestamp: String,
        task_id: String,
        title: String,
        metrics: Metrics,
    },
    Attachment {
        id: String,
        timestamp: String,
        file: AttachmentFile,
    },
}

// ---- 落库 `payload_json` 的三种形状，供回放 `from_message` 重构 ----

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolCallPayload {
    tool_name: String,
    args: serde_json::Value,
    status: ToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    task_id: String,
    stages: Vec<Stage>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultPayload {
    task_id: String,
    title: String,
    metrics: Metrics,
}

#[derive(Serialize, Deserialize)]
struct AttachmentPayload {
    file: AttachmentFile,
}

impl AppEvent {
    /// 从落库消息构造事件。role/kind 决定事件类型，结构化字段从 `payload_json` 回放；
    /// 无法解析时退回 assistant 消息，保证回放永不失败。
    pub fn from_message(msg: &Message) -> Self {
        let id = msg.id.to_string();
        let timestamp = ms_to_iso(msg.created_at_ms);
        match (msg.role.as_str(), msg.kind.as_str()) {
            ("user", _) => AppEvent::User {
                id,
                timestamp,
                content: msg.content.clone(),
            },
            ("assistant", _) => AppEvent::Assistant {
                id,
                timestamp,
                content: msg.content.clone(),
            },
            ("tool", "tool-call") => match parse_payload::<ToolCallPayload>(msg) {
                Some(p) => AppEvent::ToolCall {
                    id,
                    timestamp,
                    tool_name: p.tool_name,
                    args: p.args,
                    status: p.status,
                    summary: p.summary,
                },
                None => fallback_assistant(id, timestamp, &msg.content),
            },
            ("tool", "progress") => match parse_payload::<ProgressPayload>(msg) {
                Some(p) => AppEvent::Progress {
                    id,
                    timestamp,
                    task_id: p.task_id,
                    stages: p.stages,
                },
                None => fallback_assistant(id, timestamp, &msg.content),
            },
            ("tool", "result") => match parse_payload::<ResultPayload>(msg) {
                Some(p) => AppEvent::Result {
                    id,
                    timestamp,
                    task_id: p.task_id,
                    title: p.title,
                    metrics: p.metrics,
                },
                None => fallback_assistant(id, timestamp, &msg.content),
            },
            ("tool", "attachment") => match parse_payload::<AttachmentPayload>(msg) {
                Some(p) => AppEvent::Attachment {
                    id,
                    timestamp,
                    file: p.file,
                },
                None => fallback_assistant(id, timestamp, &msg.content),
            },
            _ => fallback_assistant(id, timestamp, &msg.content),
        }
    }
}

fn parse_payload<T: for<'de> Deserialize<'de>>(msg: &Message) -> Option<T> {
    msg.payload_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
}

fn fallback_assistant(id: String, timestamp: String, content: &str) -> AppEvent {
    AppEvent::Assistant {
        id,
        timestamp,
        content: content.to_string(),
    }
}

/// 归一化工具调用参数：`null`/缺省 → `{}`（前端 `ToolCallCard` 直接 `Object.entries`，
/// `null` 会抛异常）。
fn normalize_tool_args(args: serde_json::Value) -> serde_json::Value {
    if args.is_null() {
        serde_json::json!({})
    } else {
        args
    }
}

/// 事件「落库 + 发布」的一站式写入器：先落 `messages` 表拿到稳定 id，再据此构造
/// 事件并发布到 session 的 broadcast 通道，保证「DB 真相」与「SSE 实时」同 id。
#[derive(Clone)]
pub struct EventWriter {
    scope: AccessScope,
    session_id: i32,
    message_repo: MessageRepo,
    events: EventBus,
}

impl EventWriter {
    pub fn new(
        scope: AccessScope,
        session_id: i32,
        message_repo: MessageRepo,
        events: EventBus,
    ) -> Self {
        Self {
            scope,
            session_id,
            message_repo,
            events,
        }
    }

    /// 通用写入：落库 → 重构事件 → 发布。`task_id` 用于把 progress/result 关联到
    /// `simulation_tasks.id`（落 `messages.task_id`）。
    pub async fn emit(
        &self,
        role: &str,
        kind: &str,
        content: String,
        payload: Option<serde_json::Value>,
        task_id: Option<i32>,
    ) -> Result<AppEvent, RepoError> {
        let msg = self
            .message_repo
            .create(
                &self.scope,
                NewMessage {
                    session_id: self.session_id,
                    task_id,
                    role: role.to_string(),
                    kind: kind.to_string(),
                    content,
                    payload_json: payload.map(|p| p.to_string()),
                    created_at_ms: now_ms(),
                },
            )
            .await?;
        let event = AppEvent::from_message(&msg);
        self.events.publish(self.session_id, event.clone());
        Ok(event)
    }

    pub async fn assistant(&self, content: &str) -> Result<AppEvent, RepoError> {
        self.emit("assistant", "text", content.to_string(), None, None)
            .await
    }

    pub async fn tool_call(
        &self,
        tool_name: &str,
        args: serde_json::Value,
        status: ToolStatus,
        summary: Option<String>,
    ) -> Result<AppEvent, RepoError> {
        let args = normalize_tool_args(args);
        let content = summary.clone().unwrap_or_else(|| tool_name.to_string());
        let payload = ToolCallPayload {
            tool_name: tool_name.to_string(),
            args,
            status,
            summary,
        };
        self.emit(
            "tool",
            "tool-call",
            content,
            Some(serde_json::to_value(payload).unwrap_or_default()),
            None,
        )
        .await
    }

    pub async fn progress(&self, task_id: i32, stages: Vec<Stage>) -> Result<AppEvent, RepoError> {
        let tid = task_id.to_string();
        let payload = ProgressPayload {
            task_id: tid.clone(),
            stages,
        };
        self.emit(
            "tool",
            "progress",
            format!("进度：{tid}"),
            Some(serde_json::to_value(payload).unwrap_or_default()),
            Some(task_id),
        )
        .await
    }

    pub async fn result(
        &self,
        task_id: i32,
        title: &str,
        metrics: Metrics,
    ) -> Result<AppEvent, RepoError> {
        let tid = task_id.to_string();
        let payload = ResultPayload {
            task_id: tid,
            title: title.to_string(),
            metrics,
        };
        self.emit(
            "tool",
            "result",
            format!("结果：{title}"),
            Some(serde_json::to_value(payload).unwrap_or_default()),
            Some(task_id),
        )
        .await
    }

    pub async fn attachment(&self, file: AttachmentFile) -> Result<AppEvent, RepoError> {
        let content = format!("附件：{}", file.file_name);
        let payload = AttachmentPayload { file };
        self.emit(
            "tool",
            "attachment",
            content,
            Some(serde_json::to_value(payload).unwrap_or_default()),
            None,
        )
        .await
    }
}

/// 按 session 隔离的事件总线（每条 session 一条 broadcast 通道）。
#[derive(Clone)]
pub struct EventBus {
    senders: Arc<Mutex<HashMap<i32, broadcast::Sender<AppEvent>>>>,
}

impl EventBus {
    pub fn new() -> Self {
        Self {
            senders: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// 订阅某 session 的事件流（懒创建通道）。
    pub fn subscribe(&self, session_id: i32) -> broadcast::Receiver<AppEvent> {
        let mut senders = self.senders.lock().unwrap();
        if !senders.contains_key(&session_id) {
            let (tx, _rx) = broadcast::channel(256);
            senders.insert(session_id, tx);
        }
        senders.get(&session_id).unwrap().subscribe()
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 锁住出网字段命名，与 `frontend/src/types/index.ts`（权威）一致：
    /// `type: "tool-call"`、`toolName`、`taskId`，不得回退成 snake_case / camelCase 标签。
    #[test]
    fn wire_format_matches_frontend_types() {
        let tool_call = AppEvent::ToolCall {
            id: "1".into(),
            timestamp: "t".into(),
            tool_name: "inspect_model".into(),
            args: serde_json::json!({ "model_path": "/tmp/m.dss" }),
            status: ToolStatus::Running,
            summary: None,
        };
        let v = serde_json::to_value(tool_call).unwrap();
        assert_eq!(v["type"].as_str(), Some("tool-call"));
        assert_eq!(v["toolName"].as_str(), Some("inspect_model"));
        assert!(v.get("tool_name").is_none());

        let progress = AppEvent::Progress {
            id: "2".into(),
            timestamp: "t".into(),
            task_id: "42".into(),
            stages: vec![Stage {
                key: "sim".into(),
                label: "仿真".into(),
                status: StageStatus::Running,
            }],
        };
        let v = serde_json::to_value(progress).unwrap();
        assert_eq!(v["type"].as_str(), Some("progress"));
        assert_eq!(v["taskId"].as_str(), Some("42"));
        assert!(v.get("task_id").is_none());

        let result = AppEvent::Result {
            id: "3".into(),
            timestamp: "t".into(),
            task_id: "42".into(),
            title: "仿真结果".into(),
            metrics: Metrics {
                max_voltage: 1.02,
                min_voltage: 0.98,
                violation_nodes: 2,
                violation_duration: "0 h".into(),
            },
        };
        let v = serde_json::to_value(result).unwrap();
        assert_eq!(v["type"].as_str(), Some("result"));
        assert_eq!(v["taskId"].as_str(), Some("42"));
        assert_eq!(v["metrics"]["maxVoltage"].as_f64(), Some(1.02));
        assert!(v.get("task_id").is_none());

        let attachment = AppEvent::Attachment {
            id: "4".into(),
            timestamp: "t".into(),
            file: AttachmentFile {
                file_name: "report.pdf".into(),
                file_size: "1.2 MB".into(),
                file_type: "pdf".into(),
            },
        };
        let v = serde_json::to_value(attachment).unwrap();
        assert_eq!(v["type"].as_str(), Some("attachment"));
        assert_eq!(v["file"]["fileName"].as_str(), Some("report.pdf"));
        assert!(v["file"].get("file_name").is_none());
    }

    /// `args` 永不为 null：null → `{}`，否则原样保留。
    #[test]
    fn tool_call_args_null_normalized() {
        assert_eq!(
            normalize_tool_args(serde_json::Value::Null),
            serde_json::json!({})
        );
        let obj = serde_json::json!({ "model_path": "/tmp/m.dss" });
        assert_eq!(normalize_tool_args(obj.clone()), obj);
    }
}
