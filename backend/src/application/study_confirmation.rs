impl StudyConfigurationService {
    pub async fn prepare_confirmation(
        &self,
        user: &CurrentUser,
        configuration_id: i32,
        expected_revision: i64,
    ) -> Result<PreparedStudyConfirmation, StudyConfigurationError> {
        let aggregate = self.repo.find(&user.id, configuration_id).await?;
        if aggregate.configuration.revision != expected_revision
            || aggregate.configuration.engine_snapshot_id != self.snapshot.id
            || aggregate.configuration.status == "invalidated"
        {
            return Err(Self::invalid(
                "configuration_stale",
                "研究配置、资源或引擎快照已变化",
            ));
        }
        if aggregate.configuration.status != "ready_to_confirm" {
            return Err(Self::invalid(
                "configuration_incomplete",
                "研究函数、输入、参数或结果尚未全部确认",
            ));
        }
        let facade = self.facade(&aggregate.configuration.facade_id)?;
        self.validate_conditional_results(user, &facade, &aggregate)
            .await?;
        if facade.contract_hash != aggregate.configuration.facade_hash {
            return Err(Self::invalid("configuration_stale", "研究门面合同已经变化"));
        }
        let workspace = self
            .versions
            .workspace(&user.id, aggregate.configuration.session_id, now_ms())
            .await
            .map_err(map_version_error)?;
        let model_bindings = aggregate
            .bindings
            .iter()
            .filter(|binding| binding.input_kind == "model")
            .collect::<Vec<_>>();
        let model = if model_bindings.is_empty() {
            None
        } else {
            let active = workspace.active.ok_or_else(|| {
                Self::invalid("configuration_stale", "当前 active 模型已变化或不存在")
            })?;
            let expected = model_bindings
                .iter()
                .filter_map(|binding| binding.model_table_name.clone())
                .collect::<Vec<_>>();
            if expected.len() != model_bindings.len()
                || expected.iter().any(|name| {
                    !active
                        .dataframes
                        .iter()
                        .any(|frame| frame.table_name == *name)
                })
            {
                return Err(Self::invalid(
                    "configuration_stale",
                    "当前 active 模型不再满足研究门面",
                ));
            }
            Some(FrozenModelInput {
                model_version_id: active.version.id,
                content_hash: active.version.content_hash,
                tables: expected,
            })
        };
        let mut frozen_resources = Vec::new();
        let mut structs = Vec::new();
        let mut struct_overlays = Vec::new();
        let selected_result_names = aggregate
            .results
            .iter()
            .map(|result| result.result_name.clone())
            .collect::<Vec<_>>();
        for input in &facade.inputs {
            let binding = aggregate
                .bindings
                .iter()
                .find(|binding| binding.input_name == input.id)
                .ok_or_else(|| Self::invalid("configuration_stale", "研究输入绑定缺失"))?;
            if let Some(resource_id) = binding.resource_id.as_deref() {
                let resource = self.resources.find(&user.id, resource_id, false).await?;
                if resource.resource.status != "live"
                    || resource.resource.declared_schema_ref.as_deref()
                        != Some(input.schema_ref.as_str())
                {
                    return Err(Self::invalid(
                        "configuration_stale",
                        "绑定资源已经变化或不再兼容",
                    ));
                }
                frozen_resources.push(FrozenStudyInput {
                    input_id: input.id.clone(),
                    alias: input.alias.clone(),
                    resource_alias: resource.resource.alias,
                    kind: kind_name(input.kind).into(),
                    resource_id: Some(resource.resource.id),
                    schema_ref: input.schema_ref.clone(),
                    schema_hash: resource.resource.schema_hash,
                    content_hash: resource.normalized_sha256.clone(),
                });
            }
            if input.kind == FacadeInputKind::Struct {
                let value = aggregate
                    .structs
                    .iter()
                    .find(|value| value.input_name == input.id)
                    .ok_or_else(|| Self::invalid("configuration_stale", "Struct 参数缺失"))?;
                let resource_fields: Value =
                    serde_json::from_str(&value.values_json).map_err(|_| {
                        Self::invalid("configuration_stale", "Struct 参数持久化内容无效")
                    })?;
                let selector_fields = result_selector_fields(&facade, input)?;
                let baseline = binding
                    .mapping_json
                    .as_deref()
                    .and_then(|value| serde_json::from_str::<Value>(value).ok())
                    .and_then(|value| value.get("baselineValues").cloned())
                    .unwrap_or_else(|| resource_fields.clone());
                let user_overrides =
                    struct_user_overrides(&baseline, &resource_fields, &selector_fields)?;
                let mut effective_fields = resource_fields.clone();
                let selector_overrides = apply_result_selector_overrides(
                    &facade,
                    input,
                    &mut effective_fields,
                    &selected_result_names,
                )?;
                let mut merged_overrides = user_overrides
                    .as_object()
                    .cloned()
                    .ok_or_else(|| Self::invalid("configuration_stale", "Struct 用户覆盖无效"))?;
                for (field, value) in selector_overrides
                    .as_object()
                    .ok_or_else(|| Self::invalid("configuration_stale", "Struct 结果覆盖无效"))?
                {
                    if merged_overrides.contains_key(field) {
                        return Err(Self::invalid(
                            "configuration_stale",
                            "Struct 用户覆盖与结果覆盖冲突",
                        ));
                    }
                    merged_overrides.insert(field.clone(), value.clone());
                }
                let overrides = Value::Object(merged_overrides);
                validate_struct_values(
                    &self.snapshot,
                    &input.schema_ref,
                    &effective_fields,
                    &selector_fields,
                )?;
                let frozen = frozen_resources
                    .iter()
                    .find(|resource| resource.input_id == input.id);
                structs.push(CanonicalStructValue {
                    input_id: input.id.clone(),
                    schema_ref: input.schema_ref.clone(),
                    fields: effective_fields.clone(),
                    resource_id: frozen.and_then(|resource| resource.resource_id.clone()),
                    resource_content_hash: frozen.map(|resource| resource.content_hash.clone()),
                    resource_diff: struct_differences(binding, &resource_fields, &selector_fields),
                });
                if frozen.is_none()
                    || overrides
                        .as_object()
                        .is_some_and(|values| !values.is_empty())
                {
                    struct_overlays.push(PreparedStructOverlay {
                        input_id: input.id.clone(),
                        input_alias: input.alias.clone(),
                        source_resource_id: frozen
                            .and_then(|resource| resource.resource_id.clone()),
                        schema_ref: input.schema_ref.clone(),
                        effective_values: effective_fields,
                        user_overrides,
                        selector_overrides,
                        overrides,
                    });
                }
            }
        }
        let results = aggregate
            .results
            .iter()
            .map(|selected| {
                let result = facade
                    .results
                    .iter()
                    .find(|result| result.name == selected.result_name)
                    .expect("stored results validated against facade");
                FrozenResultContract {
                    name: result.name.clone(),
                    schema_ref: result.schema_ref.clone(),
                }
            })
            .collect();
        Ok(PreparedStudyConfirmation {
            configuration_id,
            session_id: aggregate.configuration.session_id,
            expected_revision,
            expected_snapshot_id: self.snapshot.id.clone(),
            struct_overlays,
            locked: LockedStudyContract {
                configuration_id,
                facade_id: facade.id,
                function: facade.function,
                facade_contract_hash: facade.contract_hash,
                engine_snapshot_id: self.snapshot.id.clone(),
                input_aliases: facade
                    .inputs
                    .iter()
                    .map(|input| input.alias.clone())
                    .collect(),
                structs,
                results,
            },
            inputs: WorkflowInputSnapshot {
                model,
                resources: frozen_resources,
            },
        })
    }

    async fn validate_conditional_results(
        &self,
        user: &CurrentUser,
        facade: &FacadeContract,
        aggregate: &StudyConfigurationAggregate,
    ) -> Result<(), StudyConfigurationError> {
        for constraint in &facade.result_constraints {
            let ResultSelectionRule::ConditionalUnavailable { result, when } = constraint else {
                continue;
            };
            if !aggregate
                .results
                .iter()
                .any(|selected| selected.result_name == *result)
            {
                continue;
            }
            let binding = aggregate
                .bindings
                .iter()
                .find(|binding| binding.input_name == when.input);
            let non_empty = if let Some(resource_id) =
                binding.and_then(|binding| binding.resource_id.as_deref())
            {
                self.resources
                    .find(&user.id, resource_id, false)
                    .await?
                    .resource
                    .row_count
                    > 0
            } else {
                false
            };
            if non_empty == when.non_empty {
                return Err(Self::invalid(
                    "result_selection_invalid",
                    "当前输入条件下该结果不可用",
                ));
            }
        }
        Ok(())
    }
}

