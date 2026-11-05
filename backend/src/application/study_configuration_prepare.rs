impl StudyConfigurationService {
    pub async fn prepare(
        &self,
        user: &CurrentUser,
        session_id: i32,
        request: PrepareStudyRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let facade = self.facade(&request.facade_id)?;
        let (bindings, structs, results, status) =
            self.initial_contents(user, session_id, &facade).await?;
        let intent = serde_json::json!({
            "request": request.intent,
            "derivedFromWorkflowId": request.derived_from_workflow_id,
        });
        let aggregate = self
            .repo
            .create(
                &user.id,
                NewStudyConfiguration {
                    session_id,
                    facade_id: facade.id.clone(),
                    facade_hash: facade.contract_hash.clone(),
                    engine_snapshot_id: self.snapshot.id.clone(),
                    status: status.as_str().into(),
                    intent_json: intent.to_string(),
                    bindings,
                    structs,
                    results,
                    now_ms: now_ms(),
                },
            )
            .await?;
        self.view(user, aggregate).await
    }

    pub async fn select_facade(
        &self,
        user: &CurrentUser,
        id: i32,
        request: SelectFacadeRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let current = self.repo.find(&user.id, id).await?;
        let facade = self.facade(&request.facade_id)?;
        let (bindings, structs, results, status) = self
            .initial_contents(user, current.configuration.session_id, &facade)
            .await?;
        self.replace_and_view(
            user,
            current,
            request.expected_revision,
            facade,
            bindings,
            structs,
            results,
            status,
        )
        .await
    }

    async fn initial_contents(
        &self,
        user: &CurrentUser,
        session_id: i32,
        facade: &FacadeContract,
    ) -> Result<
        (
            Vec<StudyInputBindingRecord>,
            Vec<StudyStructValueRecord>,
            Vec<StudyResultSelectionRecord>,
            StudyConfigurationState,
        ),
        StudyConfigurationError,
    > {
        let workspace = self
            .versions
            .workspace(&user.id, session_id, now_ms())
            .await
            .map_err(map_version_error)?;
        let mut bindings = Vec::new();
        let mut structs = Vec::new();
        for input in &facade.inputs {
            let mut model_table_name = None;
            let mut resource_id = None;
            let mut mapping_json = None;
            let mut selected_struct_values = None;
            match input.kind {
                FacadeInputKind::Model => {
                    model_table_name = workspace.active.as_ref().and_then(|active| {
                        active
                            .dataframes
                            .iter()
                            .find(|frame| frame.table_name == input.alias)
                            .map(|frame| frame.table_name.clone())
                    });
                }
                FacadeInputKind::Data | FacadeInputKind::Struct => {
                    let compatible = self
                        .resources
                        .compatible_for_session(&user.id, session_id, &input.schema_ref)
                        .await?;
                    let compatible = fold_selector_only_resources(
                        compatible,
                        &result_selector_fields(facade, input)?,
                    );
                    if compatible.len() == 1 {
                        let candidate_id = &compatible[0].id;
                        if input.kind == FacadeInputKind::Struct {
                            match self.struct_resource_values(user, candidate_id, input).await {
                                Ok(values) => {
                                    selected_struct_values = Some(values);
                                    resource_id = Some(candidate_id.clone());
                                }
                                Err(StudyConfigurationError::Invalid(
                                    "struct_values_invalid" | "study_binding_incompatible",
                                    _,
                                )) => {}
                                Err(error) => return Err(error),
                            }
                        } else {
                            resource_id = Some(candidate_id.clone());
                        }
                    } else if compatible.len() > 1 {
                        mapping_json =
                            Some(serde_json::json!({"selectionRequired":true}).to_string());
                    }
                }
            }
            bindings.push(StudyInputBindingRecord {
                configuration_id: 0,
                input_name: input.id.clone(),
                input_kind: kind_name(input.kind).into(),
                model_table_name,
                resource_id: resource_id.clone(),
                mapping_json: mapping_json.take(),
                ordinal: i32::try_from(input.position).unwrap_or(i32::MAX),
            });
            if input.kind == FacadeInputKind::Struct {
                let schema = self.schema(&input.schema_ref)?;
                let mut values = struct_defaults(schema);
                if let Some(selected) = resource_id.as_deref() {
                    values = if let Some(values) = selected_struct_values.take() {
                        values
                    } else {
                        self.struct_resource_values(user, selected, input).await?
                    };
                    if let Some(binding) = bindings.last_mut() {
                        binding.mapping_json = Some(struct_metadata(&values).to_string());
                    }
                }
                structs.push(StudyStructValueRecord {
                    configuration_id: 0,
                    input_name: input.id.clone(),
                    values_json: values.to_string(),
                    schema_ref: input.schema_ref.clone(),
                });
            }
        }
        let selected = default_results(facade);
        let results = selected
            .into_iter()
            .enumerate()
            .map(|(ordinal, name)| StudyResultSelectionRecord {
                configuration_id: 0,
                result_name: name,
                ordinal: i32::try_from(ordinal).unwrap_or(i32::MAX),
            })
            .collect::<Vec<_>>();
        let status = configuration_status(facade, &bindings, &structs, &results, self)?;
        Ok((bindings, structs, results, status))
    }
}
