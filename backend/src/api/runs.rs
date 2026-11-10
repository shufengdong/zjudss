//! 普通脚本运行的创建、查询、结果发现与删除 HTTP 边界。
//!
//! handler 按当前用户映射稳定 script/run ID、幂等 client run ID、分页和删除 token。输入冻结、
//! scheduler、adapter 调用、结果发布、失败诊断与 tombstone 事务由 run/dataset service 拥有；
//! API 出网前再次收敛和脱敏运行错误。

use super::ApiError;
use crate::agent::{FailureAnalysis, validate_failure_analysis};
use crate::application::diagnostics::sanitize_text;
use crate::application::{AppState, CreateRunRequest, RunSummary, ms_to_iso};
use crate::auth::CurrentUser;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::Deserialize;
use serde_json::{Value, json};

/// `POST /api/scripts/{id}/runs`：显式创建一次脚本运行。
///
/// 请求只含幂等 `clientRunId`；service 按脚本类型冻结当前输入并调度执行。首次提交返回 202，
/// 同一操作重放返回 200 和原 run，不重复执行；队列满返回 429 `run_queue_full`，输入/合同冲突
/// 返回稳定 409/422，跨用户与不存在脚本统一返回 404。
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
/// `GET /api/runs/{id}`：读取当前用户的冻结运行快照。
///
/// 成功返回 200、不可变脚本名/输入合同、阶段、时间和受控失败诊断；读取不重启或重试运行。
/// 跨用户、已 tombstone 与不存在运行统一返回 404。
pub async fn get(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(run_json(
        state.run_service.find_summary(&user, id).await?,
    )))
}
/// `POST /api/runs/{id}/cancel`：显式取消当前用户的普通脚本运行。
///
/// 排队或执行中的普通 run 原子收敛为 `cancelled_by_user`；重复取消和其他终态保持幂等并
/// 返回 200。受管 run 返回 409，要求通过 workflow 取消；跨用户、已删除与不存在 run 统一
/// 返回 404。执行中取消会通过 adapter future drop 终止整个进程组且不发布部分结果。
pub async fn cancel(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(id): Path<i32>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(run_json(state.run_service.cancel(&user, id).await?)))
}
#[derive(Deserialize)]
pub struct ListQuery {
    limit: Option<i64>,
}
/// `GET /api/sessions/{id}/runs`：列出当前用户会话的运行摘要。
///
/// `limit` 缺省 50 并由 service 限界；成功返回 200，包含每个运行冻结的脚本名与公开状态，不
/// 返回原始 stderr 或物理路径。跨用户与不存在会话统一返回 404。
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
/// `GET /api/runs/{id}/datasets`：列出运行当前仍存活的全部结果。
///
/// handler 先校验当前用户拥有 run，再返回 200 和结果元数据；已删除的单结果不再出现，该读取
/// 不扫描其他运行或创建派生资源。跨用户、已删除与不存在运行统一返回 404。
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
/// `POST /api/runs/{id}/deletion-preview`：生成终态运行删除影响与短期 token。
///
/// 成功返回 200，冻结直接结果和派生引用影响；排队/运行中返回 409 `run_busy`，被分析消费的
/// 结果返回 409 `result_busy`。该操作不删除 run，跨用户与不存在运行统一返回 404。
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
/// `DELETE /api/runs/{id}?impactToken=...`：确认删除终态运行及其直接结果。
///
/// token 必须来自当前未过期预览且影响未变化；成功在一个事务内 tombstone run 和所有存活
/// 直接结果、释放直接 artifact 引用并返回 204。无效 token、影响变化或 busy 返回稳定 409，
/// 跨用户与不存在运行统一返回 404。
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
    if code == Some("cancelled_by_user") {
        return Some(json!({"code":"cancelled_by_user","message":"用户已取消运行"}));
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
    let modern = parsed.as_ref().is_some_and(|value| {
        value.get("engineLogs").is_some()
            || value.get("analysisStatus").is_some()
            || value.get("analysis").is_some()
    });
    if modern {
        if let Some(stage) = parsed
            .as_ref()
            .and_then(|value| value.get("stage"))
            .and_then(Value::as_str)
            .and_then(|value| bounded_public_text(value, 128))
        {
            error["stage"] = Value::String(stage);
        }
        if let Some(duration_ms) = parsed
            .as_ref()
            .and_then(|value| value.get("durationMs"))
            .and_then(Value::as_i64)
            .filter(|value| *value >= 0)
        {
            error["durationMs"] = Value::from(duration_ms);
        }
        let engine_logs = parsed
            .as_ref()
            .and_then(|value| value.get("engineLogs"))
            .and_then(Value::as_array)
            .map(|logs| public_engine_logs(logs))
            .unwrap_or_default();
        let sanitized_stderr = engine_logs
            .iter()
            .filter(|log| log.get("stream").and_then(Value::as_str) == Some("stderr"))
            .filter_map(|log| log.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n");
        error["engineLogs"] = Value::Array(engine_logs);

        let stored_status = parsed
            .as_ref()
            .and_then(|value| value.get("analysisStatus"))
            .and_then(Value::as_str);
        let valid_analysis = (stored_status == Some("complete"))
            .then(|| {
                parsed
                    .as_ref()?
                    .get("analysis")
                    .cloned()
                    .and_then(|value| serde_json::from_value::<FailureAnalysis>(value).ok())
                    .filter(|analysis| {
                        validate_failure_analysis(analysis, &sanitized_stderr).is_ok()
                    })
            })
            .flatten();
        let analysis_status = match (stored_status, valid_analysis.as_ref()) {
            (Some("pending"), _) => "pending",
            (Some("complete"), Some(_)) => "complete",
            _ => "unavailable",
        };
        error["analysisStatus"] = Value::String(analysis_status.into());
        if let Some(analysis) = valid_analysis {
            error["analysis"] = serde_json::to_value(analysis).unwrap_or(Value::Null);
        }
        return Some(error);
    }
    let engine_output = parsed
        .as_ref()
        .and_then(|value| value.get("engineOutput"))
        .and_then(Value::as_str)
        .or_else(|| {
            parsed
                .as_ref()?
                .get("engineLogs")?
                .as_array()?
                .iter()
                .find(|log| log.get("stream").and_then(Value::as_str) == Some("stderr"))?
                .get("text")?
                .as_str()
        });
    if let Some(engine_output) =
        engine_output.and_then(crate::application::runs::sanitize_engine_output)
    {
        error["engineOutput"] = Value::String(engine_output);
    }
    Some(error)
}

const MAX_PUBLIC_LOG_BYTES: usize = 64 * 1024;
const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

fn bounded_public_text(value: &str, max_chars: usize) -> Option<String> {
    let sanitized = sanitize_text(value);
    let trimmed = sanitized.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut characters = trimmed.chars();
    let bounded = characters.by_ref().take(max_chars).collect::<String>();
    Some(if characters.next().is_some() {
        format!("{bounded}…")
    } else {
        bounded
    })
}

fn bounded_public_log(value: &str) -> (String, bool) {
    let sanitized = sanitize_text(value);
    if sanitized.len() <= MAX_PUBLIC_LOG_BYTES {
        return (sanitized, false);
    }
    let mut end = MAX_PUBLIC_LOG_BYTES.saturating_sub('…'.len_utf8());
    while !sanitized.is_char_boundary(end) {
        end -= 1;
    }
    (format!("{}…", &sanitized[..end]), true)
}

fn public_engine_logs(logs: &[Value]) -> Vec<Value> {
    let mut seen_stdout = false;
    let mut seen_stderr = false;
    logs.iter()
        .filter_map(|log| {
            let stream = log.get("stream").and_then(Value::as_str)?;
            let seen = match stream {
                "stdout" => &mut seen_stdout,
                "stderr" => &mut seen_stderr,
                _ => return None,
            };
            if *seen {
                return None;
            }
            *seen = true;
            let (text, locally_truncated) =
                bounded_public_log(log.get("text").and_then(Value::as_str).unwrap_or_default());
            let sent_bytes = text.len() as u64;
            let total_bytes = log
                .get("totalBytes")
                .and_then(Value::as_u64)
                .unwrap_or(sent_bytes)
                .max(sent_bytes)
                .min(MAX_SAFE_INTEGER);
            let truncated = locally_truncated
                || log
                    .get("truncated")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
            Some(json!({
                "stream": stream,
                "text": text,
                "totalBytes": total_bytes,
                "sentBytes": sent_bytes,
                "truncated": truncated
            }))
        })
        .collect()
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

    #[test]
    fn current_run_errors_expose_only_bounded_structured_diagnosis() {
        let raw = serde_json::json!({
            "code": "engine_exit_nonzero",
            "reason": "engine",
            "stage": "run",
            "durationMs": 1_234,
            "engineLogs": [{
                    "stream": "stderr",
                    "text": "/srv/private/input.rss token=secret\u{1} parse failed",
                    "fullHash": "full-secret-hash",
                    "retainedHash": "retained-secret-hash",
                    "sentHash": "sent-secret-hash",
                    "totalBytes": 61,
                    "sentBytes": 61,
                    "truncated": false,
                    "diagnostic": {"command": "tensoreval /srv/private/input.rss"}
                }, {
                    "stream": "stdout",
                    "text": "错".repeat(30_000),
                    "totalBytes": 90_000,
                    "sentBytes": 90_000,
                    "truncated": false
                }, {
                    "stream": "command",
                    "text": "tensoreval /srv/private/input.rss"
                }],
            "analysisStatus": "complete",
            "analysis": {
                "summary": "脚本解析失败",
                "facts": [{"statement": "引擎报告解析失败", "evidence": "stderr: parse failed"}],
                "possibleCauses": [],
                "unknowns": ["具体语法位置"],
                "recommendations": ["核对脚本语法"],
                "providerTrace": "private"
            },
            "serverPath": "/srv/private",
            "providerError": "secret"
        })
        .to_string();

        let public = public_run_error("failed", Some(&raw)).expect("public error");

        assert_eq!(public["code"], "run_failed");
        assert_eq!(public["stage"], "run");
        assert_eq!(public["durationMs"], 1_234);
        assert_eq!(public["analysisStatus"], "unavailable");
        assert!(public.get("analysis").is_none());
        assert_eq!(public["engineLogs"][0]["stream"], "stderr");
        assert_eq!(
            public["engineLogs"][0]["text"],
            "[服务器路径已隐藏] token=[敏感信息已隐藏] parse failed"
        );
        assert_eq!(public["engineLogs"].as_array().unwrap().len(), 2);
        assert!(public["engineLogs"][1]["text"].as_str().unwrap().len() <= 64 * 1024);
        assert_eq!(public["engineLogs"][1]["truncated"], true);
        assert!(public["engineLogs"][0].get("fullHash").is_none());
        let serialized = public.to_string();
        for private in [
            "full-secret-hash",
            "retained-secret-hash",
            "sent-secret-hash",
            "providerTrace",
            "providerError",
            "/srv/private",
        ] {
            assert!(
                !serialized.contains(private),
                "leaked {private}: {serialized}"
            );
        }
    }

    #[test]
    fn invalid_optional_analysis_does_not_hide_structured_logs() {
        let raw = serde_json::json!({
            "code": "engine_exit_nonzero",
            "reason": "solver",
            "stage": "run",
            "engineLogs": [{
                "stream": "stderr",
                "text": "solver did not converge",
                "fullHash": "full",
                "retainedHash": "retained",
                "sentHash": "sent",
                "totalBytes": 23,
                "sentBytes": 23,
                "truncated": false
            }],
            "analysisStatus": "complete",
            "analysis": {
                "summary": "求解失败",
                "facts": [{"statement": "输入不可用", "evidence": "stderr: fabricated evidence"}],
                "possibleCauses": [],
                "unknowns": [],
                "recommendations": []
            }
        })
        .to_string();

        let public = public_run_error("failed", Some(&raw)).expect("public error");

        assert_eq!(public["analysisStatus"], "unavailable");
        assert!(public.get("analysis").is_none());
        assert_eq!(public["engineLogs"][0]["text"], "solver did not converge");
    }
}
