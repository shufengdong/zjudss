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
            return Err(violation(
                vec!["function"],
                "锁定门面函数不得嵌套、间接或重复调用",
            ));
        }
        let call = &matches[0];
        if call.arguments != locked.input_aliases {
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
        let expected = locked
            .results
            .iter()
            .map(|result| result.name.clone())
            .collect::<Vec<_>>();
        if call.targets != expected {
            return Err(ContractViolation {
                fields: vec!["results".into()],
                diagnostic: json!({
                    "code":"repair_contract_violated",
                    "message":"结果绑定必须严格符合冻结合同",
                    "expected": expected,
                    "actual": call.targets,
                }),
            });
        }
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
        LockedStudyContract {
            configuration_id: 1,
            facade_id: "study".into(),
            function: "solve".into(),
            facade_contract_hash: "hash".into(),
            engine_snapshot_id: "snapshot".into(),
            input_aliases: vec!["table".into(), "para".into()],
            structs: vec![],
            results: vec![FrozenResultContract {
                name: "result".into(),
                schema_ref: "result_frames.result".into(),
            }],
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
}
