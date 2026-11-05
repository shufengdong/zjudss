impl RunService {
    pub async fn create(
        &self,
        user: &CurrentUser,
        script_id: i32,
        request: CreateRunRequest,
    ) -> Result<RunSubmission, RunError> {
        if request.client_run_id.trim().is_empty() || request.client_run_id.len() > 128 {
            return Err(RunError::Invalid(
                "clientRunId 必须为 1 到 128 个字符".into(),
            ));
        }
        let script = self.repo_for_script(user, script_id).await?;
        if let Some(created) = self
            .repo
            .find_create_replay(
                &user.id,
                script.session_id,
                script_id,
                &request.client_run_id,
            )
            .await?
        {
            return Ok(RunSubmission {
                run: created.run,
                script_name: script.name,
                replayed: true,
            });
        }
        let ticket = self
            .scheduler
            .try_admit(&user.id, script.session_id)
            .map_err(|_| RunError::QueueFull)?;
        let overlay_requests = self
            .repo
            .struct_overlay_requests(&user.id, script_id)
            .await?;
        let staged = self
            .struct_overlays
            .stage_requests(user, &overlay_requests)
            .await
            .map_err(map_rerun_overlay_error)?;
        let (inputs, drafts, mut staged_artifacts) = staged.into_parts();
        let store = self.artifacts.store().clone();
        let _publication_guard = self.artifacts.exclusive_publication_guard().await;
        let publish_store = store.clone();
        let published_new = Arc::new(Mutex::new(Vec::<String>::new()));
        let published_by_callback = published_new.clone();
        let result = self
            .repo
            .create_with_overlay_artifacts(
                &user.id,
                CreateRun {
                    script_id,
                    client_run_id: request.client_run_id,
                    engine_contract_version: ENGINE_CONTRACT_VERSION.into(),
                    created_at_ms: now_ms(),
                },
                inputs,
                drafts,
                move |index, path| {
                    let existed = publish_store
                        .resolve(path)
                        .map_err(|error| RepoError::Storage(error.to_string()))?
                        .is_file();
                    publish_store
                        .publish(&mut staged_artifacts[index], path)
                        .map_err(|error| RepoError::Storage(error.to_string()))?;
                    if !existed {
                        published_by_callback
                            .lock()
                            .expect("published overlay paths")
                            .push(path.to_string());
                    }
                    Ok(())
                },
            )
            .await;
        if result.is_err() {
            for path in published_new
                .lock()
                .expect("published overlay paths")
                .iter()
            {
                let _ = store.remove(path);
            }
        }
        let created = result?;
        if !created.replayed {
            let service = self.clone();
            let principal = user.clone();
            let run_id = created.run.id;
            let session_id = script.session_id;
            tokio::spawn(async move {
                let mut cancelled = service
                    .session_cancellations
                    .subscribe(&principal.id, session_id);
                if *cancelled.borrow() {
                    return;
                }
                let _ = ticket
                    .run_cancellable(service.execute(principal, run_id), &mut cancelled)
                    .await;
            });
        }
        Ok(RunSubmission {
            run: created.run,
            script_name: script.name,
            replayed: created.replayed,
        })
    }

    async fn repo_for_script(
        &self,
        user: &CurrentUser,
        script_id: i32,
    ) -> Result<crate::db::models::ScriptResource, RunError> {
        self.scripts
            .find(&user.id, script_id)
            .await
            .map_err(Into::into)
    }

    async fn execute(&self, user: CurrentUser, id: i32) {
        if let Err(error) = self.execute_inner(&user, id).await {
            let run = self.repo.find(&user.id, id).await.ok();
            let failed_stage = run
                .as_ref()
                .map(|run| run.stage.clone())
                .unwrap_or_else(|| "run".to_string());
            eprintln!("[run] failed id={id}, stage={failed_stage}: {error}");
            let reason = public_failure_reason(&error);
            let (_, exit_code) = engine_failure_details(&error);
            let code = match &error {
                RunError::ResultContract(code, _) => *code,
                _ => "run_failed",
            };
            let finished_at = now_ms();
            let engine_logs = match &error {
                RunError::Adapter(error) => adapter_engine_logs(error),
                _ => Vec::new(),
            };
            let failure = RunFailureDiagnosis {
                code: code.into(),
                reason: reason.into(),
                stage: failed_stage.clone(),
                duration_ms: run
                    .as_ref()
                    .and_then(|run| run.started_at_ms)
                    .map(|started| finished_at.saturating_sub(started)),
                engine_logs,
                analysis_status: if matches!(error, RunError::Adapter(_)) {
                    FailureAnalysisStatus::Pending
                } else {
                    FailureAnalysisStatus::Unavailable
                },
                analysis: None,
            };
            let failed = self
                .repo
                .fail(
                    &user.id,
                    id,
                    &failed_stage,
                    serde_json::to_string(&failure)
                        .expect("bounded run failure diagnosis serializes"),
                    exit_code,
                    finished_at,
                )
                .await;
            if failed.is_ok()
                && let Some(run) = run
            {
                let writer = EventWriter::new(
                    user.id.clone(),
                    run.session_id,
                    self.messages.clone(),
                    self.events.clone(),
                );
                let _ = writer
                    .progress(
                        OperationRef {
                            kind: OperationKind::Run,
                            id: id.to_string(),
                        },
                        run_failed_stages(&failed_stage),
                    )
                    .await;
                if let RunError::Adapter(adapter_error) = error {
                    let service = self.clone();
                    let principal = user.clone();
                    tokio::spawn(async move {
                        service
                            .enrich_failure_analysis(principal, run, adapter_error)
                            .await;
                    });
                }
            }
        }
    }

    async fn enrich_failure_analysis(
        &self,
        user: CurrentUser,
        run: SimulationRun,
        error: crate::adapters::AdapterError,
    ) {
        let analysis = async {
            let artifact_id = run.script_artifact_id?;
            let artifact = self
                .artifacts
                .find_owned(user.storage_scope_id, artifact_id)
                .await
                .ok()?;
            let path = self
                .artifacts
                .store()
                .resolve(&artifact.storage_path)
                .ok()?;
            let current_script = std::fs::read_to_string(path).ok()?;
            let request = failure_analysis_request(current_script, &error, None, None);
            self.failure_analyzer.analyze_failure(request).await.ok()
        };
        let analysis = tokio::time::timeout(std::time::Duration::from_secs(30), analysis)
            .await
            .ok()
            .flatten();
        let (status, analysis) = match analysis.as_ref() {
            Some(analysis) => (FailureAnalysisStatus::Complete, Some(analysis)),
            None => (FailureAnalysisStatus::Unavailable, None),
        };
        let _ = self
            .repo
            .complete_failure_analysis(&user.id, run.id, status, analysis)
            .await;
    }

    async fn execute_inner(&self, user: &CurrentUser, id: i32) -> Result<(), RunError> {
        self.repo
            .set_running(&user.id, id, "prepare", now_ms())
            .await?;
        let writer = EventWriter::new(
            user.id.clone(),
            self.repo.find(&user.id, id).await?.session_id,
            self.messages.clone(),
            self.events.clone(),
        );
        let _ = writer
            .progress(
                OperationRef {
                    kind: OperationKind::Run,
                    id: id.to_string(),
                },
                run_stages("prepare"),
            )
            .await;
        let context = self.repo.context(&user.id, id).await?;
        let script_artifact_id = context
            .run
            .script_artifact_id
            .ok_or_else(|| RunError::Invalid("受管脚本尚未生成".into()))?;
        let tree = build_execution_input_tree(
            user,
            &self.artifacts,
            &self.data_dir,
            "runs",
            id,
            ExecutionInputSpec {
                script_artifact_id,
                model_frames: context
                    .frames
                    .into_iter()
                    .map(|frame| ExecutionInputArtifact {
                        alias: frame.table_name,
                        artifact_id: frame.artifact_id,
                    })
                    .collect(),
                resources: Vec::new(),
            },
        )
        .await
        .map_err(map_execution_input_error)?;
        self.repo.set_running(&user.id, id, "run", now_ms()).await?;
        let _ = writer
            .progress(
                OperationRef {
                    kind: OperationKind::Run,
                    id: id.to_string(),
                },
                run_stages("run"),
            )
            .await;
        let output = self
            .engine
            .run_dataframe(RunSpec {
                mode: RunMode::Dataframe,
                input_tree: tree.input_dir.clone(),
                script_path: tree.script_path.clone(),
                named_inputs: tree.named_inputs.clone(),
                result_dir: tree.result_dir.clone(),
                timeout: self.engine_timeout,
            })
            .await
            .map_err(map_engine_run_error)?;
        let inspected = inspect_engine_outputs(&output.datasets, self.result_max_file_bytes)
            .map_err(|error| RunError::Invalid(error.to_string()))?;
        OpenResultPolicy::validate(&inspected)?;
        if let Some(contract_json) = context.run.result_contract_json.as_deref() {
            let contract: Vec<super::FrozenResultContract> = serde_json::from_str(contract_json)
                .map_err(|_| {
                    RunError::ResultContract("result_contract_invalid", "冻结结果合同无效".into())
                })?;
            FrozenResultPolicy {
                snapshot: &self.snapshot,
                contract: &contract,
            }
            .validate(&inspected)?;
        }
        let results = publish_run_results(
            &self.repo,
            &self.artifacts,
            user,
            id,
            self.result_max_file_bytes,
            inspected,
        )
        .await?;
        let _ = writer
            .progress(
                OperationRef {
                    kind: OperationKind::Run,
                    id: id.to_string(),
                },
                run_stages("completed"),
            )
            .await;
        let _ = writer
            .result(
                Some(id),
                None,
                "运行结果",
                results.iter().map(run_output_ref).collect(),
            )
            .await;
        Ok(())
    }
}
