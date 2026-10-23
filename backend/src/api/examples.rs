use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{application::AppState, auth::CurrentUser};

use super::ApiError;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExampleQuery {
    query: Option<String>,
    study_type: Option<String>,
    source_kind: Option<String>,
    limit: Option<usize>,
}

pub async fn list(
    State(state): State<AppState>,
    _: CurrentUser,
    Query(query): Query<ExampleQuery>,
) -> Json<Value> {
    let needle = query.query.unwrap_or_default().to_lowercase();
    let limit = query.limit.unwrap_or(20).clamp(1, 100);
    let examples = state
        .engine_snapshot
        .examples
        .iter()
        .filter(|example| {
            (needle.is_empty()
                || example.title.to_lowercase().contains(&needle)
                || example.id.to_lowercase().contains(&needle))
                && query
                    .study_type
                    .as_ref()
                    .is_none_or(|value| value == &example.study_type)
                && query
                    .source_kind
                    .as_ref()
                    .is_none_or(|value| value == &example.source_kind)
        })
        .take(limit)
        .map(summary)
        .collect::<Vec<_>>();
    Json(json!({
        "engineResourceSnapshot": state.engine_snapshot.id,
        "examples": examples,
        "nextCursor": null,
    }))
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
        object.insert("treeHash".into(), json!(example.tree_hash));
        object.insert(
            "files".into(),
            json!(
                example
                    .files
                    .iter()
                    .map(|file| json!({"path": file.virtual_path, "sha256": file.sha256}))
                    .collect::<Vec<_>>()
            ),
        );
        object.insert("diagnostics".into(), json!(example.diagnostics));
    }
    Ok(Json(value))
}

fn summary(example: &crate::engine_resources::EngineExample) -> Value {
    json!({
        "id": example.id,
        "title": example.title,
        "description": format!("{} · {} 个文件", example.study_type, example.file_count),
        "sourceKind": example.source_kind,
        "studyType": example.study_type,
        "solver": example.solver,
        "tags": [example.study_type, example.source_kind],
        "requiredFeatures": example.required_features,
        "fileCount": example.file_count,
        "sizeBytes": example.size_bytes,
        "verification": if example.importable { "verified" } else { "invalid" },
        "importable": example.importable,
    })
}
