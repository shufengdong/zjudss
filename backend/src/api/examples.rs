//! 当前引擎快照中的案例目录查询与案例应用入口。
//!
//! 所有路由要求当前用户。列表和详情只投影启动时发布的不可变 engine resource snapshot，不
//! 返回服务器物理路径或文件正文；应用案例委托给 case-import application service，经受控
//! projector 进入统一上传批次。本模块不拥有 manifest 校验、投影或模型/资源发布事务。

use axum::{
    Json,
    extract::{Path, Query, State},
};
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{application::AppState, auth::CurrentUser};

use super::ApiError;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleQuery {
    query: Option<String>,
    study_type: Option<String>,
}

/// `GET /api/examples`：浏览或搜索当前引擎快照的案例树。
///
/// 接受可选 `query` 与 `studyType`，为当前已认证用户返回 200、snapshot ID 和保留祖先目录的
/// 树形结果；无效案例仍可见但标记为不可导入。该读取不执行 projector 或创建上传批次。
pub async fn list(
    State(state): State<AppState>,
    _: CurrentUser,
    Query(query): Query<ExampleQuery>,
) -> Json<Value> {
    let needle = query.query.unwrap_or_default();
    let examples = state
        .engine_snapshot
        .examples
        .iter()
        .filter(|example| example.matches_catalog_query(&needle, query.study_type.as_deref()))
        .collect::<Vec<_>>();
    Json(json!({
        "engineResourceSnapshot": state.engine_snapshot.id,
        "tree": build_tree(&examples, 0, &[]),
    }))
}

#[derive(Serialize)]
#[serde(tag = "type", rename_all = "lowercase")]
enum ExampleTreeNode {
    Directory {
        name: String,
        path: Vec<String>,
        children: Vec<ExampleTreeNode>,
    },
    Example {
        example: Value,
    },
}

fn build_tree(
    examples: &[&crate::engine_resources::EngineExample],
    depth: usize,
    parent_path: &[String],
) -> Vec<ExampleTreeNode> {
    let mut directories = BTreeMap::<(String, String), Vec<_>>::new();
    let mut leaves = Vec::new();
    for example in examples {
        if let Some(name) = example.category_path.get(depth) {
            directories
                .entry((name.to_lowercase(), name.clone()))
                .or_default()
                .push(*example);
        } else {
            leaves.push(*example);
        }
    }

    let mut nodes = directories
        .into_iter()
        .map(|((_normalized, name), children)| {
            let mut path = parent_path.to_vec();
            path.push(name.clone());
            ExampleTreeNode::Directory {
                name,
                children: build_tree(&children, depth + 1, &path),
                path,
            }
        })
        .collect::<Vec<_>>();
    leaves.sort_by(|left, right| {
        left.title
            .to_lowercase()
            .cmp(&right.title.to_lowercase())
            .then_with(|| left.title.cmp(&right.title))
            .then_with(|| left.id.cmp(&right.id))
    });
    nodes.extend(leaves.into_iter().map(|example| ExampleTreeNode::Example {
        example: summary(example),
    }));
    nodes
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApplyCaseBody {
    example_id: String,
    engine_resource_snapshot: String,
    #[serde(alias = "clientImportId")]
    client_batch_id: String,
}

/// `POST /api/sessions/{id}/draft/example`：把指定快照中的案例准备为统一上传批次。
///
/// 当前用户必须拥有会话，请求携带稳定案例 ID、精确 engine snapshot ID 和幂等 client batch
/// ID。服务校验不可变清单并通过 adapter 执行受控 projector；成功返回 202 和待确认批次，
/// 不直接发布模型版本或数据资源。snapshot 漂移返回 409，案例不可投影返回稳定的 422 错误，
/// 跨用户与不存在资源统一返回 404。
pub async fn apply(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    Json(body): Json<ApplyCaseBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    let accepted = state
        .case_import_service
        .apply(
            &user,
            session_id,
            body.example_id,
            body.engine_resource_snapshot,
            body.client_batch_id,
        )
        .await?;
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({"ok":true,"uploadBatch":accepted})),
    ))
}

/// `GET /api/examples/{id}`：读取当前引擎快照中的案例详情。
///
/// 成功返回 200、规范 manifest、角色化虚拟文件哈希和诊断；只接受稳定案例 ID，不返回物理
/// 源路径或文件内容。不存在案例返回 404，该读取不创建或修改领域资源。
pub async fn get(
    State(state): State<AppState>,
    _: CurrentUser,
    Path(id): Path<String>,
) -> Result<Json<Value>, ApiError> {
    let example = state
        .engine_snapshot
        .examples
        .iter()
        .find(|example| example.id == id)
        .ok_or_else(|| ApiError::not_found("资源不存在"))?;
    let mut value = summary(example);
    if let Some(object) = value.as_object_mut() {
        object.insert("entryPath".into(), json!(example.entry_path));
        object.insert("manifest".into(), json!(example.manifest));
        object.insert("treeHash".into(), json!(example.tree_hash));
        object.insert(
            "files".into(),
            json!(
                example
                    .files
                    .iter()
                    .map(|file| json!({
                        "path": file.virtual_path,
                        "role": file.role,
                        "sha256": file.sha256,
                    }))
                    .collect::<Vec<_>>()
            ),
        );
        object.insert("diagnostics".into(), json!(example.diagnostics));
    }
    Ok(Json(value))
}

fn summary(example: &crate::engine_resources::EngineExample) -> Value {
    let study = example.manifest.as_ref().map(|manifest| &manifest.study);
    let source_entry = example
        .manifest
        .as_ref()
        .map(|manifest| manifest.source.entry.as_str());
    json!({
        "id": example.id,
        "title": example.title,
        "description": format!(
            "{} · {} 个文件",
            study.map_or("unknown", |study| study.kind.as_str()),
            example.file_count
        ),
        "sourceKind": example.source_kind,
        "categoryPath": example.category_path,
        "bundlePath": example.bundle_path,
        "studyType": study.map(|study| study.kind.as_str()),
        "solver": study.and_then(|study| study.solver.as_deref()),
        "compatibility": study.map(|study| study.compatibility.as_str()),
        "results": study.map_or_else(Vec::new, |study| study.results.clone()),
        "sourceEntry": source_entry,
        "fileCount": example.file_count,
        "sizeBytes": example.size_bytes,
        "verification": if example.importable { "verified" } else { "invalid" },
        "importable": example.importable,
        "diagnostics": example.diagnostics,
    })
}
