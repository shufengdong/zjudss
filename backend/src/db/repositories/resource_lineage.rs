use std::collections::BTreeSet;

use diesel::prelude::*;
use diesel::sql_types::{Integer, Nullable, Text};
use serde_json::{Map, Value};

use super::{RepoError, RepoResult};

const MAX_REPLACEMENT_DEPTH: usize = 64;

#[derive(Debug, Clone)]
pub(crate) struct ResolvedStructResource {
    pub id: String,
    pub artifact_id: i32,
    pub alias: String,
    pub schema_ref: String,
    pub schema_hash: String,
    pub content_hash: String,
}

#[derive(Debug, Clone)]
pub(crate) struct FrozenStructOverrides {
    pub user: Value,
    pub selector: Value,
    pub merged: Value,
}

pub(crate) fn frozen_struct_overrides(
    snapshot: &Value,
) -> RepoResult<Option<FrozenStructOverrides>> {
    let Some(effective) = snapshot.get("effective") else {
        return Ok(None);
    };
    let merged = effective
        .get("overrides")
        .cloned()
        .unwrap_or_else(|| Value::Object(Map::new()));
    let merged_object = merged
        .as_object()
        .ok_or_else(|| RepoError::InvalidState("struct_overrides_invalid".into()))?;
    let categorized =
        effective.get("userOverrides").is_some() || effective.get("selectorOverrides").is_some();
    let (user, selector) = if categorized {
        let user = effective
            .get("userOverrides")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let selector = effective
            .get("selectorOverrides")
            .cloned()
            .unwrap_or_else(|| Value::Object(Map::new()));
        let mut rebuilt = user
            .as_object()
            .cloned()
            .ok_or_else(|| RepoError::InvalidState("struct_overrides_invalid".into()))?;
        for (field, value) in selector
            .as_object()
            .ok_or_else(|| RepoError::InvalidState("struct_overrides_invalid".into()))?
        {
            if rebuilt.insert(field.clone(), value.clone()).is_some() {
                return Err(RepoError::InvalidState(
                    "struct_override_categories_overlap".into(),
                ));
            }
        }
        if rebuilt != *merged_object {
            return Err(RepoError::InvalidState(
                "struct_override_categories_mismatch".into(),
            ));
        }
        (user, selector)
    } else {
        (Value::Object(Map::new()), Value::Object(Map::new()))
    };
    if merged_object.is_empty() {
        return Ok(None);
    }
    Ok(Some(FrozenStructOverrides {
        user,
        selector,
        merged,
    }))
}

pub(crate) fn resolve_live_struct_successor(
    conn: &mut SqliteConnection,
    session_id: i32,
    frozen_resource_id: &str,
    resource_alias: &str,
    schema_ref: &str,
) -> RepoResult<Option<ResolvedStructResource>> {
    #[derive(QueryableByName)]
    struct ResourceRow {
        #[diesel(sql_type = Text)]
        id: String,
        #[diesel(sql_type = Text)]
        alias: String,
        #[diesel(sql_type = Text)]
        status: String,
        #[diesel(sql_type = Integer)]
        artifact_id: i32,
        #[diesel(sql_type = Text)]
        content_hash: String,
        #[diesel(sql_type = Nullable<Text>)]
        schema_ref: Option<String>,
        #[diesel(sql_type = Text)]
        schema_hash: String,
        #[diesel(sql_type = Nullable<Text>)]
        replaced_by_resource_id: Option<String>,
        #[diesel(sql_type = Text)]
        lineage_json: String,
    }

    let mut current_id = frozen_resource_id.to_string();
    let mut expected_parent: Option<String> = None;
    let mut visited = BTreeSet::new();
    for depth in 0..=MAX_REPLACEMENT_DEPTH {
        if !visited.insert(current_id.clone()) {
            return Err(RepoError::InvalidState("resource_replacement_cycle".into()));
        }
        let row = diesel::sql_query(
            "SELECT d.id,d.alias,d.status,d.normalized_artifact_id artifact_id,a.sha256 content_hash,d.declared_schema_ref schema_ref,d.schema_hash,d.replaced_by_resource_id,d.lineage_json \
             FROM data_resources d JOIN artifacts a ON a.id=d.normalized_artifact_id \
             WHERE d.session_id=?1 AND d.id=?2",
        )
        .bind::<Integer, _>(session_id)
        .bind::<Text, _>(&current_id)
        .get_result::<ResourceRow>(conn)
        .optional()?;
        let Some(row) = row else {
            return Ok(None);
        };
        if row.schema_ref.as_deref() != Some(schema_ref) {
            return Ok(None);
        }
        if let Some(parent) = expected_parent.as_deref() {
            let lineage: Value = serde_json::from_str(&row.lineage_json)
                .map_err(|_| RepoError::InvalidState("resource_lineage_invalid".into()))?;
            if lineage.get("replacesResourceId").and_then(Value::as_str) != Some(parent) {
                return Ok(None);
            }
        }
        if matches!(row.status.as_str(), "live" | "needs_rename") {
            if row.id != frozen_resource_id && !row.alias.eq_ignore_ascii_case(resource_alias) {
                return Ok(None);
            }
            return Ok(Some(ResolvedStructResource {
                id: row.id,
                artifact_id: row.artifact_id,
                alias: row.alias,
                schema_ref: row.schema_ref.expect("validated schema ref"),
                schema_hash: row.schema_hash,
                content_hash: row.content_hash,
            }));
        }
        if row.status != "replaced" {
            return Ok(None);
        }
        let Some(next_id) = row.replaced_by_resource_id else {
            return Ok(None);
        };
        if depth == MAX_REPLACEMENT_DEPTH {
            return Err(RepoError::InvalidState(
                "resource_replacement_depth_exceeded".into(),
            ));
        }
        expected_parent = Some(row.id);
        current_id = next_id;
    }
    unreachable!("replacement depth loop returns at its bound")
}
