impl ManagedWorkflowService {
    pub async fn preview_retry(
        &self,
        user: &CurrentUser,
        script_id: i32,
        request: RetryStudyRequest,
    ) -> Result<RetryPreviewView, WorkflowError> {
        let client_retry_id = request.client_retry_id.trim();
        if client_retry_id.is_empty() || client_retry_id.chars().count() > 128 {
            return Err(WorkflowError::Invalid(
                "client_retry_id_invalid",
                "clientRetryId 必须为 1 到 128 个字符".into(),
            ));
        }
        let script = self
            .scripts
            .find(&user.id, script_id)
            .await
            .map_err(WorkflowError::Repository)?;
        if let Some(aggregate) = self
            .repo
            .find_retry_replay(&user.id, script_id, client_retry_id)
            .await
            .map_err(map_repo_error)?
        {
            let workflow = view(aggregate);
            self.publish(&workflow);
            if self.auto_start {
                self.worker
                    .start(user.clone(), workflow.id, workflow.session_id);
            }
            return Ok(RetryPreviewView {
                retry_preview_id: None,
                changes: serde_json::json!([]),
                expires_at: None,
                workflow: Some(workflow),
            });
        }
        let contract: super::LockedStudyContract =
            serde_json::from_str(script.frozen_contract_json.as_deref().ok_or_else(|| {
                WorkflowError::Invalid("retry_not_available", "该脚本不能重新尝试".into())
            })?)
            .map_err(|_| {
                WorkflowError::Invalid("retry_not_available", "脚本研究合同无效".into())
            })?;
        let facade = self
            .snapshot
            .facades
            .get_enabled(&contract.facade_id)
            .ok_or_else(|| {
                WorkflowError::Invalid("study_contract_changed", "研究门面已不可用".into())
            })?;
        if facade.contract_hash != contract.facade_contract_hash {
            return Err(WorkflowError::Invalid(
                "study_contract_changed",
                "研究门面合同已变化".into(),
            ));
        }
        let token = generate_token();
        let token_hash = hash_token(&token);
        let now = now_ms();
        let expires = now + 5 * 60 * 1000;
        let outcome = self
            .repo
            .preview_retry(
                &user.id,
                script_id,
                client_retry_id.into(),
                self.snapshot.id.clone(),
                facade.contract_hash.clone(),
                self.correction_budget,
                self.agent_bundle_version.clone(),
                serde_json::to_string(&["study-planning@1.5.0"]).unwrap(),
                token_hash,
                expires,
                now,
            )
            .await
            .map_err(map_repo_error)?;
        let workflow = outcome.aggregate.map(view);
        if let Some(workflow) = &workflow {
            self.publish(workflow);
            if self.auto_start {
                self.worker
                    .start(user.clone(), workflow.id, workflow.session_id);
            }
        }
        Ok(RetryPreviewView {
            retry_preview_id: workflow.is_none().then_some(token),
            changes: outcome.changes,
            expires_at: workflow.is_none().then(|| ms_to_iso(expires)),
            workflow,
        })
    }

    pub async fn confirm_retry(
        &self,
        user: &CurrentUser,
        retry_preview_id: &str,
        request: ConfirmRetryRequest,
    ) -> Result<ManagedWorkflowView, WorkflowError> {
        if !request.confirmed || !is_generated_token(retry_preview_id) {
            return Err(WorkflowError::Invalid(
                "invalid_retry_preview",
                "重新尝试确认无效".into(),
            ));
        }
        let token_hash = hash_token(retry_preview_id);
        let locked_json = self
            .repo
            .retry_locked_contract(&user.id, &token_hash)
            .await
            .map_err(map_repo_error)?;
        let locked: super::LockedStudyContract =
            serde_json::from_str(&locked_json).map_err(|_| {
                WorkflowError::Invalid("invalid_retry_preview", "重新尝试合同无效".into())
            })?;
        let facade = self
            .snapshot
            .facades
            .get_enabled(&locked.facade_id)
            .ok_or_else(|| {
                WorkflowError::Invalid("study_contract_changed", "研究门面已不可用".into())
            })?;
        if facade.contract_hash != locked.facade_contract_hash {
            return Err(WorkflowError::Invalid(
                "study_contract_changed",
                "研究门面合同已变化".into(),
            ));
        }
        let overlay_requests = self
            .repo
            .retry_struct_overlay_requests(
                &user.id,
                &token_hash,
                &self.snapshot.id,
                &facade.contract_hash,
                now_ms(),
            )
            .await
            .map_err(map_repo_error)?;
        let staged = self
            .struct_overlays
            .stage_requests(user, &overlay_requests)
            .await
            .map_err(|error| WorkflowError::Invalid(error.code(), error.to_string()))?;
        let (inputs, drafts, mut staged_artifacts) = staged.into_parts();
        let store = self.artifacts.store().clone();
        let _publication_guard = self.artifacts.exclusive_publication_guard().await;
        let publish_store = store.clone();
        let published_new = Arc::new(Mutex::new(Vec::<String>::new()));
        let published_by_callback = published_new.clone();
        let result = self
            .repo
            .confirm_retry_with_overlay_artifacts(
                &user.id,
                token_hash,
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
                self.snapshot.id.clone(),
                facade.contract_hash.clone(),
                self.correction_budget,
                self.agent_bundle_version.clone(),
                serde_json::to_string(&["study-planning@1.5.0"]).unwrap(),
                now_ms(),
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
        let aggregate = result.map_err(map_repo_error)?;
        let workflow = view(aggregate);
        self.publish(&workflow);
        if self.auto_start {
            self.worker
                .start(user.clone(), workflow.id, workflow.session_id);
        }
        Ok(workflow)
    }
}

