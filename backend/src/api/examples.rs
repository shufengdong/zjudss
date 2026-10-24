use axum::{
    Json,
    extract::{Path, Query, State},
};
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::{application::AppState, auth::CurrentUser, db::models::DraftRef};

use super::ApiError;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleQuery {
    query: Option<String>,
    study_type: Option<String>,
}

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
    client_import_id: String,
    expected_draft_id: Option<i32>,
    expected_draft_revision: Option<i64>,
}

pub async fn apply(
    State(state): State<AppState>,
    user: CurrentUser,
    Path(session_id): Path<i32>,
    Json(body): Json<ApplyCaseBody>,
) -> Result<(axum::http::StatusCode, Json<Value>), ApiError> {
    if body.expected_draft_id.is_some() != body.expected_draft_revision.is_some() {
        return Err(crate::application::ModelImportError::Invalid(
            "expectedDraftId 与 expectedDraftRevision 必须同时提供".into(),
        )
        .into());
    }
    let expected = body
        .expected_draft_id
        .zip(body.expected_draft_revision)
        .map(|(id, revision)| DraftRef { id, revision });
    let service = state.case_import_service.clone();
    let accepted = service
        .accept(
            &user,
            session_id,
            &body.example_id,
            &body.engine_resource_snapshot,
            body.client_import_id,
            expected,
        )
        .await?;
    let import_id = accepted
        .id
        .parse::<i32>()
        .map_err(|_| ApiError::internal("模型导入 ID 无效"))?;
    tokio::spawn(async move {
        let _ = service.process(&user, import_id).await;
    });
    Ok((
        axum::http::StatusCode::ACCEPTED,
        Json(json!({"ok":true,"modelImport":accepted})),
    ))
}

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
