use sha2::Digest;

impl RunRepo {
    pub async fn preview_deletion(
        &self,
        user: &UserId,
        run_id: i32,
        token_hash: String,
        expires_at_ms: i64,
        now_ms: i64,
    ) -> Result<RunDeletionImpact, RunRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<RunDeletionImpact, RunRepoError, _>(|conn| {
                let run = scoped_run(conn, &user, run_id, true)?;
                if matches!(run.status.as_str(), "queued" | "running") {
                    return Err(RunRepoError::RunBusy);
                }
                let impact = run_deletion_impact(conn, &run)?;
                let impact_json = serde_json::to_string(&impact)
                    .map_err(|error| RepoError::Storage(error.to_string()))?;
                let impact_hash = hex::encode(sha2::Sha256::digest(impact_json.as_bytes()));
                diesel::sql_query("DELETE FROM run_deletion_previews WHERE expires_at_ms<?1 OR (user_id=?2 AND run_id=?3)")
                    .bind::<BigInt, _>(now_ms)
                    .bind::<Text, _>(&user)
                    .bind::<Integer, _>(run_id)
                    .execute(conn)?;
                diesel::sql_query("INSERT INTO run_deletion_previews(token_hash,user_id,run_id,impact_hash,impact_json,expires_at_ms,created_at_ms) VALUES(?1,?2,?3,?4,?5,?6,?7)")
                    .bind::<Text, _>(token_hash)
                    .bind::<Text, _>(user)
                    .bind::<Integer, _>(run_id)
                    .bind::<Text, _>(impact_hash)
                    .bind::<Text, _>(impact_json)
                    .bind::<BigInt, _>(expires_at_ms)
                    .bind::<BigInt, _>(now_ms)
                    .execute(conn)?;
                Ok(impact)
            })
        })
        .await
        .map_err(|error| RunRepoError::Repository(RepoError::Join(error)))?
    }

    pub async fn delete_run(
        &self,
        user: &UserId,
        run_id: i32,
        token_hash: String,
        now_ms: i64,
    ) -> Result<(), RunRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool.get().map_err(RepoError::Pool)?;
            conn.immediate_transaction::<(), RunRepoError, _>(|conn| {
                #[derive(QueryableByName)]
                struct Preview {
                    #[diesel(sql_type = Text)]
                    impact_hash: String,
                }
                let preview = diesel::sql_query("SELECT impact_hash FROM run_deletion_previews WHERE token_hash=?1 AND user_id=?2 AND run_id=?3 AND expires_at_ms>=?4")
                    .bind::<Text, _>(&token_hash)
                    .bind::<Text, _>(&user)
                    .bind::<Integer, _>(run_id)
                    .bind::<BigInt, _>(now_ms)
                    .get_result::<Preview>(conn)
                    .optional()?
                    .ok_or(RunRepoError::InvalidDeletionToken)?;
                let run = scoped_run(conn, &user, run_id, false)?;
                if run.deleted_at_ms.is_some() {
                    return Ok(());
                }
                if matches!(run.status.as_str(), "queued" | "running") {
                    return Err(RunRepoError::RunBusy);
                }
                let impact = run_deletion_impact(conn, &run)?;
                let impact_json = serde_json::to_string(&impact)
                    .map_err(|error| RepoError::Storage(error.to_string()))?;
                let actual_hash = hex::encode(sha2::Sha256::digest(impact_json.as_bytes()));
                if actual_hash != preview.impact_hash {
                    return Err(RunRepoError::DeletionImpactChanged);
                }
                #[derive(QueryableByName)]
                struct Count {
                    #[diesel(sql_type = BigInt)]
                    count: i64,
                }
                let busy = diesel::sql_query("SELECT count(*) count FROM result_analysis_inputs i JOIN result_analysis_runs a ON a.id=i.analysis_run_id WHERE i.dataset_id IN(SELECT id FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL) AND a.status IN('pending','running')")
                    .bind::<Integer, _>(run_id)
                    .get_result::<Count>(conn)?
                    .count;
                if busy > 0 {
                    return Err(RunRepoError::ResultBusy);
                }
                #[derive(QueryableByName)]
                struct ArtifactId {
                    #[diesel(sql_type = Integer)]
                    artifact_id: i32,
                }
                let mut artifacts = diesel::sql_query("SELECT artifact_id FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL AND artifact_id IS NOT NULL ORDER BY id")
                    .bind::<Integer, _>(run_id)
                    .load::<ArtifactId>(conn)?
                    .into_iter()
                    .map(|row| row.artifact_id)
                    .collect::<Vec<_>>();
                if let Some(artifact_id) = run.script_artifact_id {
                    artifacts.push(artifact_id);
                }
                if run.managed_workflow_id.is_none() {
                    let snapshot: FrozenRunInputs = serde_json::from_str(&run.input_snapshot_json)
                        .map_err(|error| {
                            RepoError::Storage(format!("invalid run input snapshot: {error}"))
                        })?;
                    artifacts.extend(snapshot.model.iter().flat_map(|model| {
                        model.frames.iter().map(|frame| frame.artifact_id)
                    }));
                    artifacts.extend(snapshot.resources.iter().map(|resource| resource.artifact_id));
                }
                let scope_id = schema::users::table
                    .find(&user)
                    .select(schema::users::storage_scope_id)
                    .first::<i32>(conn)?;
                super::artifact_refs::release_refs(conn, scope_id, artifacts, now_ms)?;
                let tombstone = serde_json::json!({
                    "clientRunId": run.client_run_id,
                    "status": run.status,
                    "modelContentHash": run.model_content_hash,
                    "scriptContentHash": run.script_content_hash,
                    "impact": impact
                })
                .to_string();
                diesel::sql_query("UPDATE simulation_runs SET deleted_at_ms=?1,tombstone_json=?2 WHERE id=?3 AND deleted_at_ms IS NULL")
                    .bind::<BigInt, _>(now_ms)
                    .bind::<Text, _>(tombstone)
                    .bind::<Integer, _>(run_id)
                    .execute(conn)?;
                Ok(())
            })
        })
        .await
        .map_err(|error| RunRepoError::Repository(RepoError::Join(error)))?
    }
}

fn scoped_run(
    conn: &mut SqliteConnection,
    user: &str,
    run_id: i32,
    live: bool,
) -> Result<SimulationRun, RunRepoError> {
    let mut query = schema::simulation_runs::table
        .inner_join(
            schema::sessions::table
                .on(schema::sessions::id.eq(schema::simulation_runs::session_id)),
        )
        .filter(schema::simulation_runs::id.eq(run_id))
        .filter(schema::sessions::user_id.eq(user))
        .filter(schema::sessions::deletion_status.eq("live"))
        .select(schema::simulation_runs::all_columns)
        .into_boxed();
    if live {
        query = query.filter(schema::simulation_runs::deleted_at_ms.is_null());
    }
    query
        .first(conn)
        .optional()?
        .ok_or(RunRepoError::Repository(RepoError::NotFound))
}

fn run_deletion_impact(
    conn: &mut SqliteConnection,
    run: &SimulationRun,
) -> Result<RunDeletionImpact, RunRepoError> {
    #[derive(QueryableByName)]
    struct ResultRow {
        #[diesel(sql_type = Integer)]
        id: i32,
        #[diesel(sql_type = BigInt)]
        size_bytes: i64,
    }
    let rows = diesel::sql_query("SELECT id,size_bytes FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL ORDER BY id")
        .bind::<Integer, _>(run.id)
        .load::<ResultRow>(conn)?;
    let ids = rows.iter().map(|row| row.id).collect::<Vec<_>>();
    let bytes = rows.iter().map(|row| row.size_bytes).sum();
    #[derive(QueryableByName)]
    struct Count {
        #[diesel(sql_type = BigInt)]
        count: i64,
    }
    let derived = diesel::sql_query("SELECT count(*) count FROM dataset_lineage_snapshots WHERE source_dataset_id IN(SELECT id FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL)")
        .bind::<Integer, _>(run.id)
        .get_result::<Count>(conn)?
        .count;
    let promoted = diesel::sql_query("SELECT count(*) count FROM data_resources WHERE source_kind='result' AND status<>'deleted' AND CAST(json_extract(lineage_json,'$.datasetId') AS INTEGER) IN(SELECT id FROM result_datasets WHERE run_id=?1 AND deleted_at_ms IS NULL)")
        .bind::<Integer, _>(run.id)
        .get_result::<Count>(conn)?
        .count;
    Ok(RunDeletionImpact {
        status: run.status.clone(),
        result_count: rows.len() as i64,
        result_bytes: bytes,
        derived_reference_count: derived,
        promoted_reference_count: promoted,
        result_ids: ids,
    })
}
