//! durable 事件的先落库后广播写入器。

use serde::Serialize;

use crate::auth::UserId;
use crate::db::models::NewMessage;
use crate::db::repositories::{MessageRepo, RepoError};

use super::{
    AppEvent, AttachmentFile, AttachmentPayload, ComparisonPayload, EventBus, ModelScope,
    ModelStatusPayload, ModelVersionSnapshot, OperationKind, OperationRef, ProgressPayload,
    ResultOutputRef, ResultPayload, ScriptPayload, Stage, StudyConfigCard, ToolCallPayload,
    ToolStatus, UploadBatchCard, UploadStatus, WorkflowProgressCard, now_ms,
};

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

    /// 通用写入：落库 → 重构事件 → 发布。`run_id` 用于把 progress/result 关联到
    /// `simulation_runs.id`（落 `messages.run_id`）。
    pub async fn emit(
        &self,
        role: &str,
        kind: &str,
        content: String,
        payload: Option<serde_json::Value>,
        run_id: Option<i32>,
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
                    run_id,
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

    async fn emit_operation<T: Serialize>(
        &self,
        kind: &str,
        operation_id: &str,
        content: String,
        payload: &T,
    ) -> Result<AppEvent, RepoError> {
        #[cfg(feature = "e2e-test")]
        if crate::testing::FaultInjector.take(crate::testing::FaultPoint::EventCommit) {
            return Err(RepoError::Storage(
                "injected event commit failure".to_string(),
            ));
        }
        let (msg, changed) = self
            .message_repo
            .upsert_operation_with_status(
                &self.scope,
                operation_id.to_string(),
                NewMessage {
                    session_id: self.session_id,
                    run_id: None,
                    role: "tool".into(),
                    kind: kind.into(),
                    content,
                    payload_json: Some(
                        serde_json::to_string(payload)
                            .map_err(|error| RepoError::InvalidState(error.to_string()))?,
                    ),
                    created_at_ms: now_ms(),
                },
            )
            .await?;
        let event = AppEvent::from_message(&msg);
        if changed {
            self.events.publish(self.session_id, event.clone());
        }
        Ok(event)
    }

    pub async fn upload_batch(&self, card: UploadBatchCard) -> Result<AppEvent, RepoError> {
        let operation_id = card.batch_id.clone();
        self.emit_operation(
            "upload-batch",
            &operation_id,
            format!("上传批次：{}", card.state),
            &card,
        )
        .await
    }

    pub async fn study_config(&self, card: StudyConfigCard) -> Result<AppEvent, RepoError> {
        let operation_id = card.configuration_id.clone();
        self.emit_operation(
            "study-config",
            &operation_id,
            format!("研究配置：{}", card.state),
            &card,
        )
        .await
    }

    pub async fn workflow_progress(
        &self,
        card: WorkflowProgressCard,
    ) -> Result<AppEvent, RepoError> {
        let operation_id = card.workflow_id.clone();
        self.emit_operation(
            "workflow-progress",
            &operation_id,
            format!("研究工作流：{}", card.state),
            &card,
        )
        .await
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
        let run_id = match operation.kind {
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
            run_id,
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

#[cfg(test)]
mod tests {
    use super::normalize_tool_args;

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
