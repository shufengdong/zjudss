impl ManagedWorkflowRepo {
    pub async fn cancel(
        &self,
        user: &UserId,
        workflow_id: i32,
        now_ms: i64,
    ) -> Result<ManagedWorkflowAggregate, ManagedWorkflowRepoError> {
        let pool = self.pool.clone();
        let user = user.0.clone();
        tokio::task::spawn_blocking(move || {
            let mut conn = pool
                .get()
                .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Pool(error)))?;
            conn.immediate_transaction::<_, ManagedWorkflowRepoError, _>(|conn| {
                let current = load_owned_on(conn, &user, workflow_id)?;
                if matches!(
                    current.workflow.status.as_str(),
                    "succeeded" | "failed" | "service_restarted"
                ) {
                    return Ok(current);
                }
                if current.workflow.status != "cancelled_by_user" {
                    terminalize(
                        conn,
                        &user,
                        &current,
                        None,
                        "cancelled_by_user",
                        "cancelled",
                        Some(
                            json!({"code":"cancelled_by_user","message":"用户已取消研究任务"})
                                .to_string(),
                        ),
                        now_ms,
                        &[],
                    )?;
                }
                load_owned_on(conn, &user, workflow_id)
            })
        })
        .await
        .map_err(|error| ManagedWorkflowRepoError::Repository(RepoError::Join(error)))?
    }
}

pub(crate) fn recover_interrupted_workflows(
    conn: &mut SqliteConnection,
    now_ms: i64,
) -> Result<(), diesel::result::Error> {
    #[derive(QueryableByName)]
    struct Interrupted {
        #[diesel(sql_type = diesel::sql_types::Integer)]
        id: i32,
        #[diesel(sql_type = diesel::sql_types::Text)]
        user_id: String,
    }
    let interrupted = diesel::sql_query(
        "SELECT w.id,s.user_id FROM managed_workflows w JOIN sessions s ON s.id=w.session_id \
         WHERE w.status IN ('queued','generating','executing','repairing') \
           AND s.deletion_status='live' \
         ORDER BY w.id",
    )
    .load::<Interrupted>(conn)?;
    for item in interrupted {
        conn.immediate_transaction::<_, diesel::result::Error, _>(|conn| {
            let current =
                load_owned_on(conn, &item.user_id, item.id).map_err(|error| match error {
                    ManagedWorkflowRepoError::Repository(RepoError::Query(error)) => error,
                    _ => diesel::result::Error::RollbackTransaction,
                })?;
            terminalize(
                conn,
                &item.user_id,
                &current,
                None,
                "service_restarted",
                "recovery",
                Some(
                    json!({"code":"service_restarted","message":"服务重启，研究工作流已中止"})
                        .to_string(),
                ),
                now_ms,
                &[],
            )
            .map_err(|error| match error {
                ManagedWorkflowRepoError::Repository(RepoError::Query(error)) => error,
                _ => diesel::result::Error::RollbackTransaction,
            })
        })?;
    }
    Ok(())
}

