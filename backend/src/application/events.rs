//! 事件总线与推给前端的结构化消息（对齐 `frontend/src/types/index.ts`）。

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::auth::UserId;
use crate::db::models::{Message, NewMessage};
use crate::db::repositories::{MessageRepo, RepoError};

pub const MESSAGE_TYPES: &[&str] = &[
    "user",
    "assistant",
    "tool-call",
    "progress",
    "result",
    "attachment",
    "model-status",
    "comparison",
    "script",
];

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
pub struct AttachmentFile {
    pub file_name: String,
    pub file_size: String,
    pub file_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    Ingestion,
    Run,
    Analysis,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationRef {
    pub kind: OperationKind,
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultColumn {
    pub name: String,
    pub dtype: String,
    pub nullable: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisualizationKind {
    Line,
    Bar,
    Scatter,
    Heatmap,
    Boxplot,
    Histogram,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetViewRef {
    pub id: String,
    pub name: String,
    pub kind: VisualizationKind,
    pub is_default: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResultOutputRef {
    pub dataset_id: String,
    pub name: String,
    pub row_count: i64,
    pub columns: Vec<ResultColumn>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview_rows: Option<Vec<Vec<serde_json::Value>>>,
    pub preview_truncated: bool,
    pub default_view_id: Option<String>,
    pub views: Vec<DatasetViewRef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognizedTable {
    pub input_id: String,
    pub artifact_id: String,
    pub table_kind: String,
    pub confidence: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelAmbiguity {
    pub input_id: String,
    pub candidates: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelVersionSnapshot {
    pub version_id: String,
    pub status: ModelVersionStatus,
    pub recognized_tables: Vec<RecognizedTable>,
    pub missing: Vec<String>,
    pub ambiguities: Vec<ModelAmbiguity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelVersionStatus {
    Collecting,
    Ready,
    Invalid,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelScope {
    Active,
    Draft,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UploadStatus {
    Accepted,
    Ingesting,
    Ready,
    Failed,
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
        call_id: String,
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
        operation: OperationRef,
        stages: Vec<Stage>,
    },
    #[serde(rename_all = "camelCase")]
    Result {
        id: String,
        timestamp: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        analysis_run_id: Option<String>,
        title: String,
        outputs: Vec<ResultOutputRef>,
    },
    #[serde(rename_all = "camelCase")]
    Attachment {
        id: String,
        timestamp: String,
        file: AttachmentFile,
        client_batch_id: String,
        client_file_id: String,
        ingestion_id: String,
        upload_status: UploadStatus,
    },
    #[serde(rename = "model-status", rename_all = "camelCase")]
    ModelStatus {
        id: String,
        timestamp: String,
        workspace_id: String,
        scope: ModelScope,
        snapshot: Option<ModelVersionSnapshot>,
    },
    #[serde(rename_all = "camelCase")]
    Comparison {
        id: String,
        timestamp: String,
        comparison_id: String,
        baseline_run_id: String,
        candidate_run_id: String,
        analysis_run_id: String,
        title: String,
        outputs: Vec<ResultOutputRef>,
    },
    #[serde(rename_all = "camelCase")]
    Script {
        id: String,
        timestamp: String,
        script: serde_json::Value,
    },
}

// ---- 落库 `payload_json` 的三种形状，供回放 `from_message` 重构 ----

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ToolCallPayload {
    call_id: String,
    tool_name: String,
    args: serde_json::Value,
    status: ToolStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    summary: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProgressPayload {
    operation: OperationRef,
    stages: Vec<Stage>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResultPayload {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    analysis_run_id: Option<String>,
    title: String,
    outputs: Vec<ResultOutputRef>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AttachmentPayload {
    file: AttachmentFile,
    client_batch_id: String,
    client_file_id: String,
    ingestion_id: String,
    upload_status: UploadStatus,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ModelStatusPayload {
    workspace_id: String,
    scope: ModelScope,
    snapshot: Option<ModelVersionSnapshot>,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ComparisonPayload {
    comparison_id: String,
    baseline_run_id: String,
    candidate_run_id: String,
    analysis_run_id: String,
    title: String,
    outputs: Vec<ResultOutputRef>,
}
#[derive(Serialize, Deserialize)]
struct ScriptPayload {
    script: serde_json::Value,
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
                    call_id: p.call_id,
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
                    operation: p.operation,
                    stages: p.stages,
                },
                None => fallback_assistant(id, timestamp, &msg.content),
            },
            ("tool", "result") => match parse_payload::<ResultPayload>(msg) {
                Some(p) => AppEvent::Result {
                    id,
                    timestamp,
                    run_id: p.run_id,
                    analysis_run_id: p.analysis_run_id,
                    title: p.title,
                    outputs: p.outputs,
                },
                None => fallback_assistant(id, timestamp, &msg.content),
            },
            ("tool", "attachment") => match parse_payload::<AttachmentPayload>(msg) {
                Some(p) => AppEvent::Attachment {
                    id,
                    timestamp,
                    file: p.file,
                    client_batch_id: p.client_batch_id,
                    client_file_id: p.client_file_id,
                    ingestion_id: p.ingestion_id,
                    upload_status: p.upload_status,
                },
                None => fallback_assistant(id, timestamp, &msg.content),
            },
            ("tool", "model-status") => match parse_payload::<ModelStatusPayload>(msg) {
                Some(p) => AppEvent::ModelStatus {
                    id,
                    timestamp,
                    workspace_id: p.workspace_id,
                    scope: p.scope,
                    snapshot: p.snapshot,
                },
                None => fallback_assistant(id, timestamp, &msg.content),
            },
            ("tool", "comparison") => match parse_payload::<ComparisonPayload>(msg) {
                Some(p) => AppEvent::Comparison {
                    id,
                    timestamp,
                    comparison_id: p.comparison_id,
                    baseline_run_id: p.baseline_run_id,
                    candidate_run_id: p.candidate_run_id,
                    analysis_run_id: p.analysis_run_id,
                    title: p.title,
                    outputs: p.outputs,
                },
                None => fallback_assistant(id, timestamp, &msg.content),
            },
            ("tool", "script") => match parse_payload::<ScriptPayload>(msg) {
                Some(payload) => AppEvent::Script {
                    id,
                    timestamp,
                    script: payload.script,
                },
                None => fallback_assistant(id, timestamp, &msg.content),
            },
            _ => fallback_assistant(id, timestamp, &msg.content),
        }
    }

    pub fn message_id(&self) -> Option<i32> {
        let id = match self {
            Self::User { id, .. }
            | Self::Assistant { id, .. }
            | Self::ToolCall { id, .. }
            | Self::Progress { id, .. }
            | Self::Result { id, .. }
            | Self::Attachment { id, .. }
            | Self::ModelStatus { id, .. }
            | Self::Comparison { id, .. } => id,
            Self::Script { id, .. } => id,
        };
        id.parse().ok()
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
    scope: UserId,
    session_id: i32,
    message_repo: MessageRepo,
    events: EventBus,
}

impl EventWriter {
    pub fn new(
        scope: UserId,
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
        #[cfg(feature = "e2e-test")]
        if crate::testing::FaultInjector.take(crate::testing::FaultPoint::EventCommit) {
            return Err(RepoError::Storage(
                "injected event commit failure".to_string(),
            ));
        }
        let msg = self
            .message_repo
            .create(
                &self.scope,
                NewMessage {
                    session_id: self.session_id,
                    run_id: task_id,
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

    pub async fn script(&self, script: serde_json::Value) -> Result<AppEvent, RepoError> {
        let name = script
            .get("name")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("script.rss");
        self.emit(
            "tool",
            "script",
            format!("脚本：{name}"),
            Some(serde_json::to_value(ScriptPayload { script }).unwrap_or_default()),
            None,
        )
        .await
    }


    pub async fn tool_call(
        &self,
        call_id: &str,
        tool_name: &str,
        args: serde_json::Value,
        status: ToolStatus,
        summary: Option<String>,
    ) -> Result<AppEvent, RepoError> {
        let args = normalize_tool_args(args);
        let content = summary.clone().unwrap_or_else(|| tool_name.to_string());
        let payload = ToolCallPayload {
            call_id: call_id.to_string(),
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

    pub async fn progress(
        &self,
        operation: OperationRef,
        stages: Vec<Stage>,
    ) -> Result<AppEvent, RepoError> {
        let task_id = match operation.kind {
            OperationKind::Run => operation.id.parse().ok(),
            OperationKind::Ingestion | OperationKind::Analysis => None,
        };
        let payload = ProgressPayload {
            operation: operation.clone(),
            stages,
        };
        self.emit(
            "tool",
            "progress",
            format!("进度：{}", operation.id),
            Some(serde_json::to_value(payload).unwrap_or_default()),
            task_id,
        )
        .await
    }

    pub async fn result(
        &self,
        run_id: Option<i32>,
        analysis_run_id: Option<i32>,
        title: &str,
        outputs: Vec<ResultOutputRef>,
    ) -> Result<AppEvent, RepoError> {
        let payload = ResultPayload {
            run_id: run_id.map(|id| id.to_string()),
            analysis_run_id: analysis_run_id.map(|id| id.to_string()),
            title: title.to_string(),
            outputs,
        };
        self.emit(
            "tool",
            "result",
            format!("结果：{title}"),
            Some(serde_json::to_value(payload).unwrap_or_default()),
            run_id,
        )
        .await
    }

    pub async fn attachment(
        &self,
        file: AttachmentFile,
        client_batch_id: String,
        client_file_id: String,
        ingestion_id: String,
        upload_status: UploadStatus,
    ) -> Result<AppEvent, RepoError> {
        let content = format!("附件：{}", file.file_name);
        let payload = AttachmentPayload {
            file,
            client_batch_id,
            client_file_id,
            ingestion_id,
            upload_status,
        };
        self.emit(
            "tool",
            "attachment",
            content,
            Some(serde_json::to_value(payload).unwrap_or_default()),
            None,
        )
        .await
    }

    pub async fn comparison(
        &self,
        comparison_id: i32,
        baseline_run_id: i32,
        candidate_run_id: i32,
        analysis_run_id: i32,
        title: &str,
        outputs: Vec<ResultOutputRef>,
    ) -> Result<AppEvent, RepoError> {
        let payload = ComparisonPayload {
            comparison_id: comparison_id.to_string(),
            baseline_run_id: baseline_run_id.to_string(),
            candidate_run_id: candidate_run_id.to_string(),
            analysis_run_id: analysis_run_id.to_string(),
            title: title.to_string(),
            outputs,
        };
        self.emit(
            "tool",
            "comparison",
            format!("比较：{title}"),
            Some(serde_json::to_value(payload).unwrap_or_default()),
            None,
        )
        .await
    }

    pub async fn model_status(
        &self,
        workspace_id: i32,
        scope: ModelScope,
        snapshot: Option<ModelVersionSnapshot>,
    ) -> Result<AppEvent, RepoError> {
        let payload = ModelStatusPayload {
            workspace_id: workspace_id.to_string(),
            scope,
            snapshot,
        };
        self.emit(
            "tool",
            "model-status",
            "模型工作区已更新".to_string(),
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

    /// 锁住 8 类 v2 消息及关键 camelCase 字段，与前端权威类型一致。
    #[test]
    fn wire_format_matches_frontend_types() {
        let tool_call = AppEvent::ToolCall {
            id: "1".into(),
            timestamp: "t".into(),
            call_id: "call-1".into(),
            tool_name: "inspect_model".into(),
            args: serde_json::json!({ "model_path": "/tmp/m.dss" }),
            status: ToolStatus::Running,
            summary: None,
        };
        let v = serde_json::to_value(tool_call).unwrap();
        assert_eq!(v["type"].as_str(), Some("tool-call"));
        assert_eq!(v["callId"].as_str(), Some("call-1"));
        assert_eq!(v["toolName"].as_str(), Some("inspect_model"));
        assert!(v.get("tool_name").is_none());

        let progress = AppEvent::Progress {
            id: "2".into(),
            timestamp: "t".into(),
            operation: OperationRef {
                kind: OperationKind::Run,
                id: "42".into(),
            },
            stages: vec![Stage {
                key: "sim".into(),
                label: "仿真".into(),
                status: StageStatus::Running,
            }],
        };
        let v = serde_json::to_value(progress).unwrap();
        assert_eq!(v["type"].as_str(), Some("progress"));
        assert_eq!(v["operation"]["kind"].as_str(), Some("run"));
        assert_eq!(v["operation"]["id"].as_str(), Some("42"));
        assert!(v.get("taskId").is_none());

        let result = AppEvent::Result {
            id: "3".into(),
            timestamp: "t".into(),
            run_id: Some("42".into()),
            analysis_run_id: None,
            title: "仿真结果".into(),
            outputs: vec![],
        };
        let v = serde_json::to_value(result).unwrap();
        assert_eq!(v["type"].as_str(), Some("result"));
        assert_eq!(v["runId"].as_str(), Some("42"));
        assert_eq!(v["outputs"].as_array().map(Vec::len), Some(0));
        assert!(v.get("metrics").is_none());
        assert!(v.get("taskId").is_none());

        let attachment = AppEvent::Attachment {
            id: "4".into(),
            timestamp: "t".into(),
            file: AttachmentFile {
                file_name: "report.pdf".into(),
                file_size: "1.2 MB".into(),
                file_type: "pdf".into(),
            },
            client_batch_id: "batch-1".into(),
            client_file_id: "file-1".into(),
            ingestion_id: "ingestion-1".into(),
            upload_status: UploadStatus::Accepted,
        };
        let v = serde_json::to_value(attachment).unwrap();
        assert_eq!(v["type"].as_str(), Some("attachment"));
        assert_eq!(v["file"]["fileName"].as_str(), Some("report.pdf"));
        assert!(v["file"].get("file_name").is_none());
        assert_eq!(v["clientBatchId"].as_str(), Some("batch-1"));

        let model_status = AppEvent::ModelStatus {
            id: "5".into(),
            timestamp: "t".into(),
            workspace_id: "workspace-1".into(),
            scope: ModelScope::Draft,
            snapshot: None,
        };
        let v = serde_json::to_value(model_status).unwrap();
        assert_eq!(v["type"].as_str(), Some("model-status"));
        assert_eq!(v["workspaceId"].as_str(), Some("workspace-1"));

        let comparison = AppEvent::Comparison {
            id: "6".into(),
            timestamp: "t".into(),
            comparison_id: "comparison-1".into(),
            baseline_task_id: "1".into(),
            candidate_task_id: "2".into(),
            analysis_run_id: "3".into(),
            title: "比较".into(),
            outputs: vec![],
        };
        let v = serde_json::to_value(comparison).unwrap();
        assert_eq!(v["type"].as_str(), Some("comparison"));
        assert_eq!(v["comparisonId"].as_str(), Some("comparison-1"));

        for event in [
            AppEvent::User {
                id: "7".into(),
                timestamp: "t".into(),
                content: "u".into(),
            },
            AppEvent::Assistant {
                id: "8".into(),
                timestamp: "t".into(),
                content: "a".into(),
            },
        ] {
            assert!(matches!(
                serde_json::to_value(event).unwrap()["type"].as_str(),
                Some("user" | "assistant")
            ));
        }
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
