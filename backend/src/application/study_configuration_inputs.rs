impl StudyConfigurationService {
    pub async fn bind_resource(
        &self,
        user: &CurrentUser,
        id: i32,
        input_id: &str,
        request: BindStudyResourceRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let mut aggregate = self.repo.find(&user.id, id).await?;
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        let input = facade
            .inputs
            .iter()
            .find(|input| input.id == input_id)
            .ok_or_else(|| Self::invalid("study_input_not_found", "研究输入不存在"))?;
        if input.kind == FacadeInputKind::Model {
            return Err(Self::invalid(
                "study_binding_invalid",
                "模型输入只能从当前 active 模型动态绑定",
            ));
        }
        if let Some(resource_id) = request.resource_id.as_deref() {
            let resource = self.resources.find(&user.id, resource_id, false).await?;
            if resource.resource.session_id != aggregate.configuration.session_id
                || resource.resource.status != "live"
                || resource.resource.declared_schema_ref.as_deref()
                    != Some(input.schema_ref.as_str())
            {
                return Err(Self::invalid(
                    "study_binding_incompatible",
                    "资源与研究输入 schema 或会话不兼容",
                ));
            }
        }
        let binding = aggregate
            .bindings
            .iter_mut()
            .find(|binding| binding.input_name == input_id)
            .ok_or_else(|| Self::invalid("configuration_stale", "研究输入绑定缺失"))?;
        binding.resource_id = request.resource_id;
        binding.mapping_json = None;
        if input.kind == FacadeInputKind::Struct
            && let Some(resource_id) = binding.resource_id.as_deref()
        {
            let values = self
                .struct_resource_values(user, resource_id, input)
                .await?;
            aggregate
                .structs
                .iter_mut()
                .find(|value| value.input_name == input_id)
                .expect("struct inputs always have values")
                .values_json = values.to_string();
            binding.mapping_json = Some(struct_metadata(&values).to_string());
        }
        let status = configuration_status(
            &facade,
            &aggregate.bindings,
            &aggregate.structs,
            &aggregate.results,
            self,
        )?;
        self.replace_and_view(
            user,
            aggregate,
            request.expected_revision,
            facade,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            status,
        )
        .await
    }

    pub async fn update_struct(
        &self,
        user: &CurrentUser,
        id: i32,
        input_id: &str,
        request: UpdateStudyStructRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let mut aggregate = self.repo.find(&user.id, id).await?;
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        let input = facade
            .inputs
            .iter()
            .find(|input| input.id == input_id && input.kind == FacadeInputKind::Struct)
            .ok_or_else(|| Self::invalid("study_input_not_found", "Struct 输入不存在"))?;
        let selector_fields = result_selector_fields(&facade, input)?;
        let stored = aggregate
            .structs
            .iter()
            .find(|value| value.input_name == input_id)
            .ok_or_else(|| Self::invalid("configuration_stale", "Struct 参数缺失"))?;
        let stored_values = serde_json::from_str::<Value>(&stored.values_json)
            .map_err(|_| Self::invalid("configuration_stale", "Struct 参数持久化内容无效"))?;
        let binding = aggregate
            .bindings
            .iter_mut()
            .find(|binding| binding.input_name == input_id)
            .ok_or_else(|| Self::invalid("configuration_stale", "研究输入绑定缺失"))?;
        let baseline = binding
            .mapping_json
            .as_deref()
            .and_then(|value| serde_json::from_str::<Value>(value).ok())
            .and_then(|value| value.get("baselineValues").cloned());
        let mut values = request.values;
        preserve_protected_fields(
            &mut values,
            baseline.as_ref().unwrap_or(&stored_values),
            &selector_fields,
        )?;
        validate_struct_values(&self.snapshot, &input.schema_ref, &values, &selector_fields)?;
        binding.mapping_json =
            Some(struct_metadata(baseline.as_ref().unwrap_or(&stored_values)).to_string());
        aggregate
            .structs
            .iter_mut()
            .find(|value| value.input_name == input_id)
            .expect("struct inputs always have values")
            .values_json = values.to_string();
        let status = configuration_status(
            &facade,
            &aggregate.bindings,
            &aggregate.structs,
            &aggregate.results,
            self,
        )?;
        self.replace_and_view(
            user,
            aggregate,
            request.expected_revision,
            facade,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            status,
        )
        .await
    }

    pub async fn map_resource(
        &self,
        user: &CurrentUser,
        id: i32,
        input_id: &str,
        request: MapStudyResourceRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let aggregate = self.repo.find(&user.id, id).await?;
        if aggregate.configuration.revision != request.expected_revision {
            return Err(Self::invalid(
                "configuration_stale",
                "研究配置 revision 已变化",
            ));
        }
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        let input = facade
            .inputs
            .iter()
            .find(|input| input.id == input_id && input.kind == FacadeInputKind::Data)
            .ok_or_else(|| Self::invalid("study_input_not_found", "Data 输入不存在"))?;
        let mapped = self
            .data_mapping
            .create(
                user,
                CreateMappedResourceRequest {
                    session_id: aggregate.configuration.session_id,
                    source_resource_id: request.source_resource_id,
                    display_name: request.display_name,
                    alias: request.alias,
                    target_schema_ref: input.schema_ref.clone(),
                    mappings: request.mappings,
                },
                self.schema(&input.schema_ref)?,
            )
            .await?;
        self.bind_resource(
            user,
            id,
            input_id,
            BindStudyResourceRequest {
                expected_revision: request.expected_revision,
                resource_id: Some(mapped.id),
            },
        )
        .await
    }

    pub async fn select_results(
        &self,
        user: &CurrentUser,
        id: i32,
        request: SelectStudyResultsRequest,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let mut aggregate = self.repo.find(&user.id, id).await?;
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        aggregate.results = request
            .results
            .into_iter()
            .enumerate()
            .map(|(ordinal, result_name)| StudyResultSelectionRecord {
                configuration_id: id,
                result_name,
                ordinal: i32::try_from(ordinal).unwrap_or(i32::MAX),
            })
            .collect();
        validate_results(&facade, &aggregate.results)?;
        self.validate_conditional_results(user, &facade, &aggregate)
            .await?;
        let status = configuration_status(
            &facade,
            &aggregate.bindings,
            &aggregate.structs,
            &aggregate.results,
            self,
        )?;
        self.replace_and_view(
            user,
            aggregate,
            request.expected_revision,
            facade,
            Vec::new(),
            Vec::new(),
            Vec::new(),
            status,
        )
        .await
    }

    async fn struct_resource_values(
        &self,
        user: &CurrentUser,
        resource_id: &str,
        input: &FacadeInputContract,
    ) -> Result<Value, StudyConfigurationError> {
        let page = self
            .data_resources
            .rows(user, resource_id, Some(2), None, Vec::new())
            .await
            .map_err(map_struct_resource_error)?;
        if page.rows.is_empty() {
            return Ok(struct_defaults(self.schema(&input.schema_ref)?));
        }
        if page.rows.len() != 1 {
            return Err(Self::invalid(
                "study_binding_incompatible",
                "Struct 资源必须恰好包含一行",
            ));
        }
        let values = page
            .columns
            .iter()
            .cloned()
            .zip(page.rows[0].iter().cloned())
            .collect::<serde_json::Map<_, _>>();
        let values = Value::Object(values);
        validate_struct_values(&self.snapshot, &input.schema_ref, &values, &[])?;
        Ok(values)
    }

    #[allow(clippy::too_many_arguments)]
    async fn replace_and_view(
        &self,
        user: &CurrentUser,
        aggregate: StudyConfigurationAggregate,
        expected_revision: i64,
        facade: FacadeContract,
        bindings: Vec<StudyInputBindingRecord>,
        structs: Vec<StudyStructValueRecord>,
        results: Vec<StudyResultSelectionRecord>,
        status: StudyConfigurationState,
    ) -> Result<StudyConfigurationView, StudyConfigurationError> {
        let bindings = if bindings.is_empty() {
            aggregate.bindings
        } else {
            bindings
        };
        let structs = if structs.is_empty() {
            aggregate.structs
        } else {
            structs
        };
        let results = if results.is_empty() {
            aggregate.results
        } else {
            results
        };
        let updated = self
            .repo
            .replace(
                &user.id,
                aggregate.configuration.id,
                expected_revision,
                facade.id.clone(),
                facade.contract_hash.clone(),
                self.snapshot.id.clone(),
                status.as_str().into(),
                bindings,
                structs,
                results,
                now_ms(),
            )
            .await?;
        self.view(user, updated).await
    }
}
