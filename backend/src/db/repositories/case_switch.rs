//! A case is installed as one coherent model/resource set.
use diesel::prelude::*;
use diesel::sql_types::{BigInt, Integer, Text};

use super::data_resource::owned_batch;
use super::upload_confirmation::UploadConfirmationRepo;
use super::upload_confirmation::transactions::{
    apply_model_candidates_to_draft_on, publish_resources_on,
};
use super::upload_confirmation::{DraftApplicationMode, PublishChildResult, ResourcePublishInput};
use super::version::{PublishCommand, VersionRepoError, publish_on};
use super::{RepoError, RepoResult};
use crate::auth::UserId;
use crate::db::models::{DraftRef, UploadCandidateRecord};
use crate::db::schema;

impl UploadConfirmationRepo {
    pub async fn switch_case(
        &self,
        user: &UserId,
        batch_id: i32,
        mut inputs: Vec<ResourcePublishInput>,
        expected_draft: Option<DraftRef>,
        now_ms: i64,
    ) -> RepoResult<PublishChildResult> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction(|conn| {
                let batch = owned_batch(conn, &user, batch_id)?;
                if batch.source_kind != "case" || batch.model_publish_status == "published" || batch.resource_publish_status == "published" {
                    return Err(RepoError::InvalidState("请重新从案例库准备完整的案例切换批次".into()));
                }
                let diagnostics: serde_json::Value = serde_json::from_str(&batch.diagnostics_json).unwrap_or_default();
                let current = case_switch_state_on(conn, &user, batch.session_id)?;
                if diagnostics.get("caseSwitch") != Some(&current) {
                    return Err(RepoError::InvalidState("案例切换影响已变化，请重新从案例库准备并确认".into()));
                }
                let candidates = schema::upload_candidates::table.filter(schema::upload_candidates::batch_id.eq(batch_id)).load::<UploadCandidateRecord>(conn)?;
                for candidate in &candidates {
                    let evidence: serde_json::Value = serde_json::from_str(&candidate.evidence_json).unwrap_or_default();
                    let purpose = if evidence["inputKind"] == "model" { "model_table" } else { "data_resource" };
                    if !candidate.selected || candidate.purpose.as_deref() != Some(purpose) || candidate.normalized_alias.as_deref() != evidence["alias"].as_str() {
                        return Err(RepoError::InvalidState("案例必须按目录角色整体切换，不能跳过或修改输入用途".into()));
                    }
                }
                // Follow replacement chains so user-edited case parameters retire with their case.
                let old = case_resources_on(conn, batch.session_id, batch_id)?;
                for input in &mut inputs {
                    if let Some(previous) = old.iter().find(|old| old.normalized_alias == input.normalized_alias) {
                        let mut lineage: serde_json::Value = serde_json::from_str(&input.lineage_json).unwrap_or_default();
                        lineage["caseReplacement"] = serde_json::json!({"previousResourceId":previous.id});
                        input.lineage_json = lineage.to_string();
                    }
                }
                for old in &old {
                    diesel::update(schema::data_resources::table.find(&old.id)).set((schema::data_resources::status.eq("replaced"),schema::data_resources::updated_at_ms.eq(now_ms))).execute(conn)?;
                    diesel::sql_query("UPDATE study_configurations SET status='invalidated',updated_at_ms=?1 WHERE id IN(SELECT configuration_id FROM study_input_bindings WHERE resource_id=?2) AND status IN ('choosing_facade','collecting_inputs','ready_to_confirm')")
                        .bind::<BigInt,_>(now_ms).bind::<Text,_>(&old.id).execute(conn)?;
                }
                apply_model_candidates_to_draft_on(
                    conn,
                    &user,
                    batch_id,
                    expected_draft,
                    DraftApplicationMode::ReplaceAll,
                    now_ms,
                )?;
                let resources = publish_resources_on(conn, &user, batch_id, inputs, None, now_ms)?;
                for previous in &old {
                    if let Some(next) = resources.resources.iter().find(|next| next.normalized_alias == previous.normalized_alias) {
                        diesel::update(schema::data_resources::table.find(&previous.id)).set(schema::data_resources::replaced_by_resource_id.eq(Some(&next.id))).execute(conn)?;
                    }
                }
                #[derive(QueryableByName)] struct Draft { #[diesel(sql_type=Integer)] id:i32, #[diesel(sql_type=BigInt)] revision:i64 }
                let draft = diesel::sql_query("SELECT d.id,d.revision FROM workspace_drafts d JOIN workspaces w ON w.id=d.workspace_id WHERE w.session_id=?1")
                    .bind::<Integer,_>(batch.session_id).get_result::<Draft>(conn)?;
                let scope_id = schema::users::table.find(&user).select(schema::users::storage_scope_id).first::<i32>(conn)?;
                let published = publish_on(conn, &user, batch.session_id, PublishCommand {
                    expected: DraftRef { id:draft.id, revision:draft.revision }, client_publish_id:format!("case-batch-{batch_id}"),
                    description:diagnostics["caseProjection"]["title"].as_str().unwrap_or("切换应用案例").chars().take(200).collect(), engine_contract:"tensoreval-parquet-v3".into(), now_ms,
                }).map_err(|error| match error { VersionRepoError::Repository(error) => error, _ => RepoError::InvalidState("案例模型版本发布失败".into()) })?;
                super::artifact_refs::release_refs(
                    conn,
                    scope_id,
                    published.release_artifact_ids,
                    now_ms,
                )?;
                diesel::update(schema::study_configurations::table.filter(schema::study_configurations::session_id.eq(batch.session_id)).filter(schema::study_configurations::status.eq_any(["choosing_facade","collecting_inputs","ready_to_confirm"])))
                    .set((schema::study_configurations::status.eq("invalidated"),schema::study_configurations::updated_at_ms.eq(now_ms))).execute(conn)?;
                let receipt = serde_json::json!({"versionId":published.version_id,"activated":true,"deduplicated":published.deduplicated,"caseBatchId":batch_id});
                diesel::update(schema::upload_batches::table.find(batch_id)).set(schema::upload_batches::model_receipt_json.eq(Some(receipt.to_string()))).execute(conn)?;
                Ok(resources)
            })
        }).await.map_err(RepoError::Join)?
    }
    pub async fn case_switch_state(
        &self,
        user: &UserId,
        session_id: i32,
    ) -> RepoResult<serde_json::Value> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.transaction(|conn| case_switch_state_on(conn, &user, session_id))
        })
        .await
        .map_err(RepoError::Join)?
    }
}

fn case_switch_state_on(
    conn: &mut diesel::SqliteConnection,
    user: &str,
    session_id: i32,
) -> RepoResult<serde_json::Value> {
    super::data_resource::ensure_session(conn, user, session_id)?;
    #[derive(QueryableByName, serde::Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Workspace {
        #[diesel(sql_type=diesel::sql_types::Nullable<Integer>)]
        active_version_id: Option<i32>,
        #[diesel(sql_type=diesel::sql_types::Nullable<Integer>)]
        draft_id: Option<i32>,
        #[diesel(sql_type=diesel::sql_types::Nullable<BigInt>)]
        draft_revision: Option<i64>,
    }
    let workspace = diesel::sql_query("SELECT w.active_version_id,d.id draft_id,d.revision draft_revision FROM workspaces w LEFT JOIN workspace_drafts d ON d.workspace_id=w.id WHERE w.session_id=?1")
        .bind::<Integer,_>(session_id).get_result::<Workspace>(conn).optional()?;
    let workspace = workspace
        .map(|value| serde_json::to_value(value).expect("workspace serializes"))
        .unwrap_or_else(
            || serde_json::json!({"activeVersionId":null,"draftId":null,"draftRevision":null}),
        );
    let resources = schema::data_resources::table
        .filter(schema::data_resources::session_id.eq(session_id))
        .filter(schema::data_resources::status.eq_any(["live", "needs_rename"]))
        .order(schema::data_resources::id.asc())
        .select((
            schema::data_resources::id,
            schema::data_resources::normalized_alias,
        ))
        .load::<(String, String)>(conn)?;
    let retiring = case_resources_on(conn, session_id, 0)?;
    let previous = schema::upload_batches::table
        .filter(schema::upload_batches::session_id.eq(session_id))
        .filter(schema::upload_batches::source_kind.eq("case"))
        .filter(schema::upload_batches::model_publish_status.eq("published"))
        .order((
            schema::upload_batches::updated_at_ms.desc(),
            schema::upload_batches::id.desc(),
        ))
        .select((
            schema::upload_batches::id,
            schema::upload_batches::diagnostics_json,
        ))
        .first::<(i32, String)>(conn)
        .optional()?;
    let previous = previous.map(|(id,raw)| {
        let value:serde_json::Value=serde_json::from_str(&raw).unwrap_or_default();
        serde_json::json!({"batchId":id,"title":value["caseProjection"]["title"],"exampleId":value["caseProjection"]["exampleId"]})
    });
    Ok(
        serde_json::json!({"workspace":workspace,"resources":resources,"retiringResources":retiring,"previousCase":previous}),
    )
}

#[derive(QueryableByName, serde::Serialize)]
#[serde(rename_all = "camelCase")]
struct CaseResource {
    #[diesel(sql_type=Text)]
    id: String,
    #[diesel(sql_type=Text)]
    normalized_alias: String,
    #[diesel(sql_type=Text)]
    display_name: String,
}
fn case_resources_on(
    conn: &mut diesel::SqliteConnection,
    session_id: i32,
    excluded_batch: i32,
) -> RepoResult<Vec<CaseResource>> {
    Ok(diesel::sql_query("WITH RECURSIVE case_ids(id) AS (SELECT r.id FROM data_resources r JOIN upload_batches b ON b.id=r.source_batch_id WHERE r.session_id=?1 AND b.source_kind='case' AND b.id<>?2 UNION SELECT r.replaced_by_resource_id FROM data_resources r JOIN case_ids c ON r.id=c.id WHERE r.session_id=?1 AND r.replaced_by_resource_id IS NOT NULL) SELECT r.id,r.normalized_alias,r.display_name FROM data_resources r JOIN case_ids c ON r.id=c.id WHERE r.session_id=?1 AND r.status IN ('live','needs_rename') ORDER BY r.id")
        .bind::<Integer,_>(session_id).bind::<Integer,_>(excluded_batch).load(conn)?)
}
