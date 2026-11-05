use std::collections::BTreeSet;

use serde_json::Value;

use crate::catalog::{DataTypeContract, SchemaFieldContract};
use crate::engine_resources::{
    EngineResourceSnapshot, FacadeContract, FacadeInputContract, ResultSelectionRule,
};

use super::StudyConfigurationError;

fn invalid(code: &'static str, message: impl Into<String>) -> StudyConfigurationError {
    StudyConfigurationError::Invalid(code, message.into())
}

pub(crate) fn json_values_equivalent(left: &Value, right: &Value) -> bool {
    if left == right {
        return true;
    }
    let (Some(left), Some(right)) = (left.as_number(), right.as_number()) else {
        return false;
    };
    if let (Some(left), Some(right)) = (left.as_i64(), right.as_i64()) {
        return left == right;
    }
    if let (Some(left), Some(right)) = (left.as_u64(), right.as_u64()) {
        return left == right;
    }
    left.as_f64()
        .zip(right.as_f64())
        .is_some_and(|(left, right)| left == right)
}

pub(crate) fn result_selector_fields(
    facade: &FacadeContract,
    input: &FacadeInputContract,
) -> Result<Vec<String>, StudyConfigurationError> {
    let mut fields = Vec::new();
    for constraint in &facade.result_constraints {
        let ResultSelectionRule::OrderedUniqueSelection { selector, .. } = constraint else {
            continue;
        };
        let Some((selector_input, field)) = selector.split_once('.') else {
            return Err(invalid("configuration_stale", "结果选择器格式无效"));
        };
        if selector_input == input.alias || selector_input == input.id {
            fields.push(field.to_string());
        }
    }
    fields.sort();
    fields.dedup();
    Ok(fields)
}

pub(crate) fn apply_result_selector_overrides(
    facade: &FacadeContract,
    input: &FacadeInputContract,
    fields: &mut Value,
    selected_results: &[String],
) -> Result<Value, StudyConfigurationError> {
    let object = fields
        .as_object_mut()
        .ok_or_else(|| invalid("struct_values_invalid", "Struct 参数必须是对象"))?;
    let mut overrides = serde_json::Map::new();
    for field in result_selector_fields(facade, input)? {
        let value = Value::String(selected_results.join(","));
        object.insert(field.clone(), value.clone());
        overrides.insert(field, value);
    }
    Ok(Value::Object(overrides))
}

pub(crate) fn preserve_protected_fields(
    values: &mut Value,
    baseline: &Value,
    fields: &[String],
) -> Result<(), StudyConfigurationError> {
    let object = values
        .as_object_mut()
        .ok_or_else(|| invalid("struct_values_invalid", "Struct 参数必须是对象"))?;
    let baseline = baseline
        .as_object()
        .ok_or_else(|| invalid("configuration_stale", "Struct 参数基线无效"))?;
    for field in fields {
        if let Some(value) = baseline.get(field) {
            object.insert(field.clone(), value.clone());
        } else {
            object.remove(field);
        }
    }
    Ok(())
}

pub(crate) fn validate_struct_values(
    snapshot: &EngineResourceSnapshot,
    schema_ref: &str,
    values: &Value,
    _protected_fields: &[String],
) -> Result<(), StudyConfigurationError> {
    let schema = snapshot
        .schemas
        .resolve(schema_ref)
        .map_err(|error| invalid("study_schema_unavailable", error.to_string()))?;
    let object = values
        .as_object()
        .ok_or_else(|| invalid("struct_values_invalid", "Struct 参数必须是对象"))?;
    let known = schema
        .fields
        .iter()
        .map(|field| field.name.as_str())
        .collect::<BTreeSet<_>>();
    if object.keys().any(|name| !known.contains(name.as_str())) {
        return Err(invalid("struct_values_invalid", "Struct 参数包含未知字段"));
    }
    for field in &schema.fields {
        let value = object.get(&field.name);
        if field.required && value.is_none() {
            return Err(invalid("struct_values_invalid", "Struct 参数缺少必填字段"));
        }
        if let Some(value) = value {
            validate_field(field, value)?;
        }
    }
    Ok(())
}

fn validate_field(
    field: &SchemaFieldContract,
    value: &Value,
) -> Result<(), StudyConfigurationError> {
    if value.is_null() && field.nullable {
        return Ok(());
    }
    let valid_type = match &field.dtype {
        DataTypeContract::Primitive { name } | DataTypeContract::Semantic { name } => {
            match name.as_str() {
                "str" | "string" => value.is_string(),
                "bool" => value.is_boolean(),
                name if name.starts_with('u') || name.starts_with('i') => {
                    value.as_i64().is_some() || value.as_u64().is_some()
                }
                _ => value.is_number(),
            }
        }
        DataTypeContract::Constructor { .. } => value.is_array(),
    };
    if !valid_type {
        return Err(invalid("struct_values_invalid", "Struct 参数类型不匹配"));
    }
    if let Some(card) = &field.card {
        if card.enum_values.state == "values"
            && !card.enum_values.values.iter().any(|candidate| {
                candidate == value
                    || (card.enum_values.case_insensitive
                        && candidate
                            .as_str()
                            .zip(value.as_str())
                            .is_some_and(|(left, right)| left.eq_ignore_ascii_case(right)))
            })
        {
            return Err(invalid(
                "struct_values_invalid",
                "Struct 参数不在允许枚举中",
            ));
        }
        if card.range.state == "bounded" {
            let number = value
                .as_f64()
                .ok_or_else(|| invalid("struct_values_invalid", "范围约束要求数值"))?;
            if let Some(minimum) = card.range.minimum.as_ref().and_then(Value::as_f64) {
                let inclusive = card.range.minimum_inclusive.unwrap_or(true);
                if number < minimum || (!inclusive && number == minimum) {
                    return Err(invalid("struct_values_invalid", "Struct 参数低于最小值"));
                }
            }
            if let Some(maximum) = card.range.maximum.as_ref().and_then(Value::as_f64) {
                let inclusive = card.range.maximum_inclusive.unwrap_or(true);
                if number > maximum || (!inclusive && number == maximum) {
                    return Err(invalid("struct_values_invalid", "Struct 参数高于最大值"));
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use serde_json::{Value, json};

    use crate::catalog::SchemaFieldContract;
    use crate::engine_resources::{
        EngineResourceSnapshot, FacadeContract, FacadeInputContract, FacadeInputKind,
        ResultSelectionRule,
    };

    use super::{
        apply_result_selector_overrides, json_values_equivalent, preserve_protected_fields,
        result_selector_fields, validate_field, validate_struct_values,
    };

    fn input() -> FacadeInputContract {
        FacadeInputContract {
            id: "parameters".into(),
            alias: "para".into(),
            kind: FacadeInputKind::Struct,
            schema_ref: "frames.para".into(),
            required: true,
            position: 1,
        }
    }

    fn facade() -> FacadeContract {
        FacadeContract {
            id: "policy-test".into(),
            function: "policy_test".into(),
            family: "test".into(),
            title: "测试".into(),
            description: "策略测试".into(),
            differences: vec![],
            inputs: vec![input()],
            results: vec![],
            result_constraints: vec![
                ResultSelectionRule::OrderedUniqueSelection {
                    selector: "para.z_result".into(),
                    minimum: 1,
                    binding_count: "one".into(),
                },
                ResultSelectionRule::OrderedUniqueSelection {
                    selector: "parameters.a_result".into(),
                    minimum: 1,
                    binding_count: "one".into(),
                },
                ResultSelectionRule::OrderedUniqueSelection {
                    selector: "para.a_result".into(),
                    minimum: 1,
                    binding_count: "one".into(),
                },
            ],
            contract_hash: "policy-hash".into(),
        }
    }

    fn field(value: Value) -> SchemaFieldContract {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn numeric_json_equivalence_accepts_integer_and_float_spellings() {
        assert!(json_values_equivalent(&json!(50), &json!(50.0)));
        assert!(!json_values_equivalent(&json!(50), &json!(50.5)));
    }

    #[test]
    fn selector_fields_are_sorted_deduplicated_and_applied_in_result_order() {
        let facade = facade();
        let input = input();
        assert_eq!(
            result_selector_fields(&facade, &input).unwrap(),
            vec!["a_result", "z_result"]
        );
        let mut values = json!({"kept":true});
        let overrides = apply_result_selector_overrides(
            &facade,
            &input,
            &mut values,
            &["summary".into(), "bus_voltage".into()],
        )
        .unwrap();
        assert_eq!(
            overrides,
            json!({
                "a_result":"summary,bus_voltage",
                "z_result":"summary,bus_voltage"
            })
        );
    }

    #[test]
    fn protected_selector_fields_are_restored_from_the_server_baseline() {
        let mut values = json!({"result":"forged","max_iter":31});
        preserve_protected_fields(
            &mut values,
            &json!({"result":"bus_voltage","max_iter":30}),
            &["result".into()],
        )
        .unwrap();
        assert_eq!(values, json!({"result":"bus_voltage","max_iter":31}));
    }

    #[test]
    fn struct_validation_rejects_unknown_fields_and_unavailable_schema() {
        let snapshot = EngineResourceSnapshot::minimal(
            "policy-snapshot",
            "policy-contract",
            PathBuf::from("policy-test"),
        );
        let unknown = validate_struct_values(&snapshot, "frames.para", &json!({"unknown":1}), &[])
            .unwrap_err();
        assert_eq!(unknown.code(), "struct_values_invalid");
        let unavailable =
            validate_struct_values(&snapshot, "frames.missing", &json!({}), &[]).unwrap_err();
        assert_eq!(unavailable.code(), "study_schema_unavailable");
    }

    #[test]
    fn field_validation_enforces_nullability_and_numeric_ranges() {
        let non_nullable = field(json!({
            "name":"count", "dtype":{"kind":"primitive","name":"u32"},
            "required":true, "nullable":false, "unit":"not_applicable"
        }));
        assert_eq!(
            validate_field(&non_nullable, &Value::Null)
                .unwrap_err()
                .code(),
            "struct_values_invalid"
        );
        let bounded = field(json!({
            "name":"ratio", "dtype":{"kind":"primitive","name":"f64"},
            "required":false, "nullable":false, "unit":"not_applicable",
            "card":{
                "default":{"state":"omitted"},
                "unit":{"state":"not_applicable"},
                "range":{"state":"bounded","minimum":0,"maximum":1,"minimumInclusive":false,"maximumInclusive":true},
                "enum":{"state":"not_applicable"},
                "visibleWhen":{"state":"always"},
                "enabledWhen":{"state":"always"}
            }
        }));
        assert_eq!(
            validate_field(&bounded, &json!(0)).unwrap_err().to_string(),
            "Struct 参数低于最小值"
        );
        assert!(validate_field(&bounded, &json!(1)).is_ok());
    }

    #[test]
    fn list_and_matrix_fields_reject_non_array_shapes() {
        for dtype in [
            json!({"kind":"constructor","name":"list","element":{"kind":"primitive","name":"f64"}}),
            json!({"kind":"constructor","name":"matrix","element":{"kind":"constructor","name":"list","element":{"kind":"primitive","name":"f64"}}}),
        ] {
            let constructor = field(json!({
                "name":"shape", "dtype":dtype, "required":false, "nullable":false,
                "unit":"not_applicable"
            }));
            assert_eq!(
                validate_field(&constructor, &json!({"not":"an array"}))
                    .unwrap_err()
                    .code(),
                "struct_values_invalid"
            );
        }
    }
}
