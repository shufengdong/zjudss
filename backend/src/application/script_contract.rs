use serde::Serialize;
use serde_json::{Value, json};

use super::LockedStudyContract;
use super::managed_rss::{
    assigned_identifiers, called_functions, direct_assignment_calls, tokenize,
};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ContractViolation {
    pub fields: Vec<String>,
    pub diagnostic: Value,
}

pub struct ScriptContractInspector;

impl ScriptContractInspector {
    pub fn inspect_candidate(
        source: &str,
        locked: &LockedStudyContract,
    ) -> Result<(), ContractViolation> {
        let tokens = tokenize(source).map_err(|message| violation(vec!["syntax"], message))?;
        let calls = called_functions(&tokens);
        let assigned = assigned_identifiers(&tokens);
        let overwritten = locked
            .input_aliases
            .iter()
            .filter(|alias| assigned.contains(alias))
            .cloned()
            .collect::<Vec<_>>();
        if !overwritten.is_empty() {
            eprintln!("[contract_violation] inputAliases overwritten: {:?}", overwritten);
            return Err(ContractViolation {
                fields: vec!["inputAliases".into()],
                diagnostic: json!({
                    "code":"repair_contract_violated",
                    "message":"冻结输入 alias 不得在脚本中重新赋值",
                    "aliases":overwritten,
                }),
            });
        }
        if calls.iter().any(|name| {
            matches!(
                name.as_str(),
                "import_dss" | "import_bmopf" | "read" | "read_csv" | "read_parquet"
            )
        }) {
            eprintln!("[contract_violation] imports found");
            return Err(violation(
                vec!["imports"],
                "受管脚本不得自行导入模型或服务器文件",
            ));
        }
        let matches = direct_assignment_calls(&tokens)
            .into_iter()
            .filter(|call| call.callee == locked.function)
            .collect::<Vec<_>>();
        if matches.len() != 1 {
            eprintln!("[contract_violation] function: expected 1 direct call to {}, found {}", locked.function, matches.len());
            return Err(violation(
                vec!["function"],
                "脚本必须且只能直接调用一次锁定门面函数",
            ));
        }
        if calls
            .iter()
            .filter(|name| *name == &locked.function)
            .count()
            != 1
        {
            eprintln!("[contract_violation] function: {} called {} times", locked.function, calls.iter().filter(|n| *n == &locked.function).count());
            return Err(violation(
                vec!["function"],
                "锁定门面函数不得嵌套、间接或重复调用",
            ));
        }
        let call = &matches[0];
        if call.arguments != locked.input_aliases {
            eprintln!("[contract_violation] arguments mismatch ({} vs {})", call.arguments.len(), locked.input_aliases.len());
            eprintln!("  expected: {:?}", locked.input_aliases);
            eprintln!("  actual:   {:?}", call.arguments);
            let expected_str = locked.input_aliases.join(", ");
            let actual_str = call.arguments.join(", ");
            if expected_str != actual_str {
                eprintln!("  expected_str: {}", expected_str);
                eprintln!("  actual_str:   {}", actual_str);
            }
            return Err(ContractViolation {
                fields: vec!["inputAliases".into()],
                diagnostic: json!({
                    "code":"repair_contract_violated",
                    "message":"门面输入必须严格使用冻结 alias 和顺序",
                    "expected": locked.input_aliases,
                    "actual": call.arguments,
                }),
            });
        }
        let expected_count = locked.results.len();
        if call.targets_bracketed {
            eprintln!("[contract_violation] targets_bracketed: expected false, got true; targets={:?}", call.targets);
            return Err(ContractViolation {
                fields: vec!["results".into()],
                diagnostic: json!({
                    "code":"repair_contract_violated",
                    "message":"单 DataFrame 返回值不得使用方括号解构",
                    "targets": call.targets,
                }),
            });
        }
        if call.targets.len() != 1 {
            eprintln!("[contract_violation] targets_count: expected 1, got {} (expected_results={})", call.targets.len(), expected_count);
            return Err(ContractViolation {
                fields: vec!["results".into()],
                diagnostic: json!({
                    "code":"repair_contract_violated",
                    "message":"引擎函数始终返回单个 DataFrame，脚本左值必须是单个标识符",
                    "targets": call.targets,
                }),
            });
        }
        eprintln!("[contract_ok] function={}, args={} targets={}", locked.function, call.arguments.len(), call.targets.len());
        Ok(())
    }
}

fn violation(fields: Vec<&str>, message: impl Into<String>) -> ContractViolation {
    ContractViolation {
        fields: fields.into_iter().map(str::to_string).collect(),
        diagnostic: json!({
            "code":"repair_contract_violated",
            "message":message.into(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::FrozenResultContract;

    fn locked() -> LockedStudyContract {
        locked_with_results(&["result"])
    }

    fn locked_with_results(names: &[&str]) -> LockedStudyContract {
        LockedStudyContract {
            configuration_id: 1,
            facade_id: "study".into(),
            function: "solve".into(),
            facade_contract_hash: "hash".into(),
            engine_snapshot_id: "snapshot".into(),
            input_aliases: vec!["table".into(), "para".into()],
            structs: vec![],
            results: names
                .iter()
                .map(|name| FrozenResultContract {
                    name: (*name).into(),
                    schema_ref: format!("result_frames.{name}"),
                })
                .collect(),
        }
    }

    #[test]
    fn accepts_exact_direct_facade_call_with_helpers() {
        let source = "helper = select(table); result = solve(table, para); result;";
        ScriptContractInspector::inspect_candidate(source, &locked()).unwrap();
    }

    #[test]
    fn rejects_imports_changed_inputs_and_hidden_call() {
        for source in [
            "x=import_dss(\"model.dss\"); result=solve(table,para);",
            "result=solve(para,table);",
            "result=wrap(solve(table,para));",
            "table=select(table); result=solve(table,para);",
            "if(ok){table=select(table);} result=solve(table,para);",
            "para=with_columns(para); result=solve(table,para);",
        ] {
            assert!(ScriptContractInspector::inspect_candidate(source, &locked()).is_err());
        }
    }

    #[test]
    fn rejects_bracketed_binding_even_with_multiple_results() {
        let locked = locked_with_results(&["first", "second"]);
        let violation = ScriptContractInspector::inspect_candidate(
            "[first, second] = solve(table, para);",
            &locked,
        )
        .unwrap_err();
        assert_eq!(violation.diagnostic["code"], "repair_contract_violated");
        assert!(
            violation.diagnostic["message"]
                .as_str()
                .unwrap()
                .contains("方括号")
        );

        let violation = ScriptContractInspector::inspect_candidate(
            "first, second = solve(table, para);",
            &locked,
        )
        .unwrap_err();
        assert_eq!(violation.diagnostic["code"], "repair_contract_violated");
    }

    #[test]
    fn accepts_single_target_even_when_multiple_results_declared() {
        let locked = locked_with_results(&["execution.bus_voltage", "fault_study.fault_result"]);
        ScriptContractInspector::inspect_candidate(
            "result = solve(table, para);",
            &locked,
        )
        .unwrap();
    }
}