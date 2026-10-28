use super::ApiError;
use crate::application::{AppState, CreateRunRequest, RunSummary, ms_to_iso};
use crate::auth::CurrentUser;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

pub async fn create(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(script_id): Path<i32>,
    Json(request): Json<CreateRunRequest>,
) -> Result<(StatusCode, Json<Value>), ApiError> {
    let submitted = state.run_service.create(&user, script_id, request).await?;
    let status = if submitted.replayed {
        StatusCode::OK
    } else {
        StatusCode::ACCEPTED
    };
    Ok((
        status,
        Json(
            json!({"run":run_json(RunSummary { run: submitted.run, script_name: submitted.script_name }),"replayed":submitted.replayed}),
        ),
    ))
}
pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(run_json(
        state.run_service.find_summary(&user, id).await?,
    )))
}
#[derive(Deserialize)]
pub struct ListQuery {
    limit: Option<i64>,
}
pub async fn list(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    Query(query): Query<ListQuery>,
) -> Result<Json<Value>, ApiError> {
    let runs = state
        .run_service
        .list_summaries(&user, session_id, query.limit.unwrap_or(50))
        .await?;
    Ok(Json(
        json!({"runs":runs.into_iter().map(run_json).collect::<Vec<_>>() }),
    ))
}
pub async fn datasets(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    state.run_service.find(&user, id).await?;
    Ok(Json(
        json!({"datasets":state.dataset_service.list(&user,id).await?}),
    ))
}
pub async fn deletion_preview(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        serde_json::to_value(state.run_service.preview_deletion(&user, id).await?)
            .map_err(|error| ApiError::internal(error.to_string()))?,
    ))
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeleteQuery {
    impact_token: String,
}
pub async fn delete(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
    Query(query): Query<DeleteQuery>,
) -> Result<StatusCode, ApiError> {
    state
        .run_service
        .delete(&user, id, &query.impact_token)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}
fn run_json(summary: RunSummary) -> Value {
    let RunSummary { run, script_name } = summary;
    let public_error = public_run_error(&run.status, run.error_json.as_deref());
    json!({"id":run.id.to_string(),"sessionId":run.session_id.to_string(),"modelVersionId":run.model_version_id.map(|id|id.to_string()),"modelContentHash":run.model_content_hash,"scriptResourceId":run.script_resource_id.map(|id|id.to_string()),"scriptName":script_name,"scriptArtifactId":run.script_artifact_id.map(|id|id.to_string()),"scriptContentHash":run.script_content_hash,"scriptMessageId":run.script_message_id.map(|id|id.to_string()),"managedWorkflowId":run.managed_workflow_id.map(|id|id.to_string()),"clientRunId":run.client_run_id,"status":run.status,"stage":run.stage,"engineContractVersion":run.engine_contract_version,"agentBundleVersion":run.agent_bundle_version,"agentSkillRefs":serde_json::from_str::<Value>(&run.agent_skill_refs_json).unwrap_or_else(|_|json!([])),"exitCode":run.exit_code,"error":public_error,"createdAt":ms_to_iso(run.created_at_ms),"startedAt":run.started_at_ms.map(ms_to_iso),"finishedAt":run.finished_at_ms.map(ms_to_iso)})
}

fn public_run_error(status: &str, raw: Option<&str>) -> Option<Value> {
    if status != "failed" {
        return None;
    }
    let parsed = raw.and_then(|value| serde_json::from_str::<Value>(value).ok());
    let code = parsed
        .as_ref()
        .and_then(|value| value.get("code"))
        .and_then(Value::as_str);
    if code == Some("service_restarted") {
        return Some(json!({"code":"service_restarted","message":"服务重启，运行已中止"}));
    }
    let reason = parsed
        .as_ref()
        .and_then(|value| value.get("reason"))
        .and_then(Value::as_str);
    let message = match reason {
        Some("model") => "模型或脚本不符合计算要求",
        Some("solver") => "求解失败或未收敛",
        Some("timeout") => "计算超时",
        Some("io") => "计算输入输出不可用",
        Some("result") => "计算结果校验失败",
        Some("engine") => "仿真引擎执行失败",
        Some("internal") => "计算服务发生内部错误",
        _ => "运行失败，未提供可用原因",
    };
    let public_code = match code {
        Some("result_contract_invalid") => "result_contract_invalid",
        Some("no_result_dataset") => "no_result_dataset",
        _ => "run_failed",
    };
    let mut error = json!({"code":public_code,"message":message});
    if let Some(engine_output) = parsed
        .as_ref()
        .and_then(|value| value.get("engineOutput"))
        .and_then(Value::as_str)
        .and_then(crate::application::runs::sanitize_engine_output)
    {
        error["engineOutput"] = Value::String(engine_output);
    }
    Some(error)
}

#[cfg(test)]
mod tests {
    use super::public_run_error;
    use serde_json::json;

    #[test]
    fn legacy_run_errors_never_expose_stored_diagnostics() {
        let raw = r#"{"code":"run_failed","message":"failed at /srv/private/input.rss"}"#;
        assert_eq!(
            public_run_error("failed", Some(raw)),
            Some(json!({
                "code": "run_failed",
                "message": "运行失败，未提供可用原因"
            }))
        );
    }

    #[test]
    fn current_run_errors_return_bounded_sanitized_engine_output() {
        let raw = serde_json::json!({
            "code": "run_failed",
            "reason": "engine",
            "engineOutput": "RSS parse error\nat /srv/private/input.rss:2:1"
        })
        .to_string();
        assert_eq!(
            public_run_error("failed", Some(&raw)),
            Some(json!({
                "code": "run_failed",
                "message": "仿真引擎执行失败",
                "engineOutput": "RSS parse error\nat [服务器路径已隐藏]"
            }))
        );

        let oversized = serde_json::json!({
            "code": "run_failed",
            "reason": "engine",
            "engineOutput": "错".repeat(17_000)
        })
        .to_string();
        let output = public_run_error("failed", Some(&oversized)).unwrap()["engineOutput"]
            .as_str()
            .unwrap()
            .to_string();
        assert_eq!(output.chars().count(), 16_385);
        assert!(output.ends_with('…'));
    }
}
