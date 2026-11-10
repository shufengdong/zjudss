//! durable 消息的前端事件投影、写入器与 session 广播入口（对齐 `frontend/src/types/index.ts`）。

use serde::{Deserialize, Serialize};

use crate::db::models::Message;

mod bus;
mod writer;

pub use bus::EventBus;
pub use writer::EventWriter;

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
    "upload-batch",
    "study-config",
    "workflow-progress",
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
    pub created_at: i64,
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
    pub views: Vec<DatasetViewRef>,
}

impl ResultOutputRef {
    pub(crate) fn from_dataset(dataset: &crate::db::repositories::RunDataset) -> Self {
        let columns: Vec<crate::application::datasets::DatasetColumn> =
            serde_json::from_str(&dataset.schema_json).unwrap_or_default();
        Self {
            dataset_id: dataset.id.to_string(),
            name: dataset.result_name.clone(),
            row_count: dataset.row_count,
            columns: columns
                .into_iter()
                .map(|column| ResultColumn {
                    name: column.name,
                    dtype: column.dtype,
                    nullable: column.nullable,
                    unit: column.unit,
                })
                .collect(),
            preview_rows: None,
            preview_truncated: dataset.row_count > 0,
            views: Vec::new(),
        }
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadCandidateCard {
    pub id: String,
    pub name: String,
    pub state: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadBatchCard {
    pub batch_id: String,
    pub batch_revision: i64,
    pub state: String,
    pub candidates: Vec<UploadCandidateCard>,
    pub model_transaction_state: String,
    pub resource_transaction_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub diagnostic: Option<EventDiagnostic>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyConfigCard {
    pub configuration_id: String,
    pub revision: i64,
    pub state: String,
    pub action_card: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowAttemptCard {
    pub execution_index: u32,
    pub repair_index: u32,
    pub state: String,
    pub elapsed_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowProgressCard {
    pub workflow_id: String,
    pub state: String,
    pub execution_index: u32,
    pub execution_max: u32,
    pub repair_index: u32,
    pub repair_max: u32,
    pub elapsed_ms: i64,
    pub attempts: Vec<WorkflowAttemptCard>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_id: Option<String>,
    #[serde(default)]
    pub output_dataset_ids: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
}

/// 推给前端的结构化消息（discriminated union，`type` 区分）。
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AppEvent {
    User {
        id: String,
        timestamp: String,
        revision: i32,
        content: String,
    },
    Assistant {
        id: String,
        timestamp: String,
        revision: i32,
        content: String,
    },
    #[serde(rename = "tool-call", rename_all = "camelCase")]
    ToolCall {
        id: String,
        timestamp: String,
        revision: i32,
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
        revision: i32,
        operation: OperationRef,
        stages: Vec<Stage>,
    },
    #[serde(rename_all = "camelCase")]
    Result {
        id: String,
        timestamp: String,
        revision: i32,
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
        revision: i32,
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
        revision: i32,
        workspace_id: String,
        scope: ModelScope,
        snapshot: Option<ModelVersionSnapshot>,
    },
    #[serde(rename_all = "camelCase")]
    Comparison {
        id: String,
        timestamp: String,
        revision: i32,
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
        revision: i32,
        script: serde_json::Value,
    },
    #[serde(rename = "upload-batch", rename_all = "camelCase")]
    UploadBatch {
        id: String,
        timestamp: String,
        revision: i32,
        card: UploadBatchCard,
    },
    #[serde(rename = "study-config", rename_all = "camelCase")]
    StudyConfig {
        id: String,
        timestamp: String,
        revision: i32,
        card: StudyConfigCard,
    },
    #[serde(rename = "workflow-progress", rename_all = "camelCase")]
    WorkflowProgress {
        id: String,
        timestamp: String,
        revision: i32,
        card: WorkflowProgressCard,
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
                revision: msg.revision,
                content: msg.content.clone(),
            },
            ("assistant", _) => AppEvent::Assistant {
                id,
                timestamp,
                revision: msg.revision,
                content: msg.content.clone(),
            },
            ("tool", "tool-call") => match parse_payload::<ToolCallPayload>(msg) {
                Some(p) => AppEvent::ToolCall {
                    id,
                    timestamp,
                    revision: msg.revision,
                    call_id: p.call_id,
                    tool_name: p.tool_name,
                    args: p.args,
                    status: p.status,
                    summary: p.summary,
                },
                None => fallback_assistant(id, timestamp, msg.revision, &msg.content),
            },
            ("tool", "progress") => match parse_payload::<ProgressPayload>(msg) {
                Some(p) => AppEvent::Progress {
                    id,
                    timestamp,
                    revision: msg.revision,
                    operation: p.operation,
                    stages: p.stages,
                },
                None => fallback_assistant(id, timestamp, msg.revision, &msg.content),
            },
            ("tool", "result") => match parse_payload::<ResultPayload>(msg) {
                Some(p) => AppEvent::Result {
                    id,
                    timestamp,
                    revision: msg.revision,
                    run_id: p.run_id,
                    analysis_run_id: p.analysis_run_id,
                    title: p.title,
                    outputs: p.outputs,
                },
                None => fallback_assistant(id, timestamp, msg.revision, &msg.content),
            },
            ("tool", "attachment") => match parse_payload::<AttachmentPayload>(msg) {
                Some(p) => AppEvent::Attachment {
                    id,
                    timestamp,
                    revision: msg.revision,
                    file: p.file,
                    client_batch_id: p.client_batch_id,
                    client_file_id: p.client_file_id,
                    ingestion_id: p.ingestion_id,
                    upload_status: p.upload_status,
                },
                None => fallback_assistant(id, timestamp, msg.revision, &msg.content),
            },
            ("tool", "model-status") => match parse_payload::<ModelStatusPayload>(msg) {
                Some(p) => AppEvent::ModelStatus {
                    id,
                    timestamp,
                    revision: msg.revision,
                    workspace_id: p.workspace_id,
                    scope: p.scope,
                    snapshot: p.snapshot,
                },
                None => fallback_assistant(id, timestamp, msg.revision, &msg.content),
            },
            ("tool", "comparison") => match parse_payload::<ComparisonPayload>(msg) {
                Some(p) => AppEvent::Comparison {
                    id,
                    timestamp,
                    revision: msg.revision,
                    comparison_id: p.comparison_id,
                    baseline_run_id: p.baseline_run_id,
                    candidate_run_id: p.candidate_run_id,
                    analysis_run_id: p.analysis_run_id,
                    title: p.title,
                    outputs: p.outputs,
                },
                None => fallback_assistant(id, timestamp, msg.revision, &msg.content),
            },
            ("tool", "script") => match parse_payload::<ScriptPayload>(msg) {
                Some(payload) => AppEvent::Script {
                    id,
                    timestamp,
                    revision: msg.revision,
                    script: payload.script,
                },
                None => fallback_assistant(id, timestamp, msg.revision, &msg.content),
            },
            ("tool", "upload-batch") => match parse_payload::<UploadBatchCard>(msg) {
                Some(card) => AppEvent::UploadBatch {
                    id,
                    timestamp,
                    revision: msg.revision,
                    card,
                },
                None => fallback_assistant(id, timestamp, msg.revision, &msg.content),
            },
            ("tool", "study-config") => match parse_payload::<StudyConfigCard>(msg) {
                Some(card) => AppEvent::StudyConfig {
                    id,
                    timestamp,
                    revision: msg.revision,
                    card,
                },
                None => fallback_assistant(id, timestamp, msg.revision, &msg.content),
            },
            ("tool", "workflow-progress") => match parse_payload::<WorkflowProgressCard>(msg) {
                Some(card) => AppEvent::WorkflowProgress {
                    id,
                    timestamp,
                    revision: msg.revision,
                    card,
                },
                None => fallback_assistant(id, timestamp, msg.revision, &msg.content),
            },
            _ => fallback_assistant(id, timestamp, msg.revision, &msg.content),
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
            | Self::Comparison { id, .. }
            | Self::Script { id, .. }
            | Self::UploadBatch { id, .. }
            | Self::StudyConfig { id, .. }
            | Self::WorkflowProgress { id, .. } => id,
        };
        id.parse().ok()
    }

    pub fn revision(&self) -> i32 {
        match self {
            Self::User { revision, .. }
            | Self::Assistant { revision, .. }
            | Self::ToolCall { revision, .. }
            | Self::Progress { revision, .. }
            | Self::Result { revision, .. }
            | Self::Attachment { revision, .. }
            | Self::ModelStatus { revision, .. }
            | Self::Comparison { revision, .. }
            | Self::Script { revision, .. }
            | Self::UploadBatch { revision, .. }
            | Self::StudyConfig { revision, .. }
            | Self::WorkflowProgress { revision, .. } => *revision,
        }
    }

    pub fn event_id(&self) -> Option<String> {
        Some(format!("{}:{}", self.message_id()?, self.revision()))
    }
}

fn parse_payload<T: for<'de> Deserialize<'de>>(msg: &Message) -> Option<T> {
    msg.payload_json
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
}

fn fallback_assistant(id: String, timestamp: String, revision: i32, content: &str) -> AppEvent {
    AppEvent::Assistant {
        id,
        timestamp,
        revision,
        content: content.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_output_ref_uses_the_shared_dataset_mapping() {
        let dataset = crate::db::repositories::RunDataset {
            id: 7,
            run_id: Some(2),
            analysis_run_id: None,
            artifact_id: Some(3),
            result_name: "voltage".into(),
            schema_json: serde_json::json!([{
                "name": "v_mag",
                "dtype": "Float64",
                "nullable": false,
                "unit": "pu"
            }])
            .to_string(),
            schema_hash: "schema".into(),
            row_count: 12,
            size_bytes: 128,
            created_at_ms: 1,
            deleted_at_ms: None,
            tombstone_json: None,
        };

        let output = ResultOutputRef::from_dataset(&dataset);

        assert_eq!(output.dataset_id, "7");
        assert_eq!(output.name, "voltage");
        assert_eq!(output.row_count, 12);
        assert_eq!(output.columns[0].name, "v_mag");
        assert_eq!(output.columns[0].unit.as_deref(), Some("pu"));
        assert!(output.preview_truncated);
        assert!(output.views.is_empty());
    }

    #[test]
    fn malformed_structured_payload_falls_back_without_losing_identity() {
        let message = Message {
            id: 42,
            session_id: 1,
            run_id: None,
            operation_id: None,
            revision: 3,
            role: "tool".into(),
            kind: "result".into(),
            content: "fallback".into(),
            payload_json: Some("{invalid".into()),
            created_at_ms: 0,
        };

        let event = AppEvent::from_message(&message);

        assert_eq!(event.event_id().as_deref(), Some("42:3"));
        match event {
            AppEvent::Assistant {
                id,
                revision,
                content,
                ..
            } => {
                assert_eq!(id, "42");
                assert_eq!(revision, 3);
                assert_eq!(content, "fallback");
            }
            other => panic!("expected assistant fallback, got {other:?}"),
        }
    }

    /// 锁住消息类型及关键 camelCase 字段，与前端权威类型一致。
    #[test]
    fn wire_format_matches_frontend_types() {
        let tool_call = AppEvent::ToolCall {
            id: "1".into(),
            timestamp: "t".into(),
            revision: 1,
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
            revision: 1,
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
            revision: 1,
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
            revision: 1,
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
            revision: 1,
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
            revision: 1,
            comparison_id: "comparison-1".into(),
            baseline_run_id: "1".into(),
            candidate_run_id: "2".into(),
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
                revision: 1,
                content: "u".into(),
            },
            AppEvent::Assistant {
                id: "8".into(),
                timestamp: "t".into(),
                revision: 1,
                content: "a".into(),
            },
        ] {
            assert!(matches!(
                serde_json::to_value(event).unwrap()["type"].as_str(),
                Some("user" | "assistant")
            ));
        }
    }
}
