use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use crate::application::managed_rss::{Token, tokenize};

#[derive(Debug, Clone, PartialEq)]
pub(super) struct AnalysisScriptViolation {
    pub code: &'static str,
    pub fields: Vec<String>,
    pub diagnostic: Value,
}

pub(super) fn inspect_analysis_script(
    source: &str,
    input_aliases: &[String],
    expected_outputs: &[String],
    functions: &Value,
) -> Result<(), AnalysisScriptViolation> {
    let tokens = tokenize(source).map_err(|message| invalid("tokenize", "script", message))?;
    if tokens.is_empty() || tokens.last() != Some(&Token::Semicolon) {
        return Err(invalid(
            "structure",
            "script",
            "every declaration must end with a semicolon",
        ));
    }
    let catalog = FunctionCatalog::parse(functions)?;
    validate_calls(&tokens, &catalog)?;
    let statements = statements(&tokens)?;
    let inputs = input_aliases
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();
    let expected = expected_outputs
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>();
    if expected.iter().copied().collect::<HashSet<_>>().len() != expected.len() {
        return Err(invalid(
            "outputs",
            "expectedOutputs",
            "expected outputs must be unique",
        ));
    }

    let mut assignments = Vec::with_capacity(statements.len());
    let mut targets = HashSet::new();
    for statement in statements {
        let assignment = parse_assignment(statement)?;
        for target in &assignment.targets {
            let normalized = target.to_ascii_lowercase();
            if inputs.contains(target.as_str()) {
                return Err(invalid(
                    "assignment",
                    "alias",
                    format!("input alias '{target}' cannot be rebound"),
                ));
            }
            if catalog.names.contains(&normalized) {
                return Err(invalid(
                    "assignment",
                    "alias",
                    format!("target '{target}' conflicts with a function name"),
                ));
            }
            if !targets.insert(target.clone()) {
                return Err(invalid(
                    "assignment",
                    "alias",
                    format!("target '{target}' is assigned more than once"),
                ));
            }
        }
        assignments.push(assignment);
    }

    let mut referenced = HashSet::new();
    for assignment in &assignments {
        for token in assignment.rhs {
            if let Token::Ident(name) = token
                && targets.contains(name)
            {
                referenced.insert(name.clone());
            }
        }
    }
    let terminal = assignments
        .iter()
        .flat_map(|assignment| assignment.targets.iter())
        .filter(|target| !referenced.contains(*target))
        .map(String::as_str)
        .collect::<Vec<_>>();
    if terminal != expected {
        return Err(AnalysisScriptViolation {
            code: "analysis_script_invalid",
            fields: vec!["expectedOutputs".into(), "script".into()],
            diagnostic: json!({
                "stage": "outputs",
                "expected": expected,
                "actual": terminal,
            }),
        });
    }
    Ok(())
}

struct Assignment<'a> {
    targets: Vec<String>,
    rhs: &'a [Token],
}

fn statements(tokens: &[Token]) -> Result<Vec<&[Token]>, AnalysisScriptViolation> {
    let mut result = Vec::new();
    let mut start = 0;
    let mut depth = 0_i32;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen | Token::LBracket => depth += 1,
            Token::RParen | Token::RBracket => {
                depth -= 1;
                if depth < 0 {
                    return Err(invalid("structure", "script", "unbalanced delimiter"));
                }
            }
            Token::Semicolon if depth == 0 => {
                if start == index {
                    return Err(invalid("structure", "script", "empty declaration"));
                }
                result.push(&tokens[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if depth != 0 || start != tokens.len() {
        return Err(invalid(
            "structure",
            "script",
            "unbalanced or unterminated declaration",
        ));
    }
    Ok(result)
}

fn parse_assignment(statement: &[Token]) -> Result<Assignment<'_>, AnalysisScriptViolation> {
    let mut depth = 0_i32;
    let equals = statement
        .iter()
        .enumerate()
        .filter_map(|(index, token)| {
            match token {
                Token::LParen | Token::LBracket => depth += 1,
                Token::RParen | Token::RBracket => depth -= 1,
                _ => {}
            }
            (depth == 0 && token == &Token::Equal).then_some(index)
        })
        .collect::<Vec<_>>();
    if equals.len() != 1 {
        return Err(invalid(
            "structure",
            "script",
            "each declaration must contain one top-level assignment",
        ));
    }
    let equal = equals[0];
    let targets = parse_targets(&statement[..equal])?;
    let rhs = &statement[equal + 1..];
    let (Some(Token::Ident(_)), Some(Token::LParen)) = (rhs.first(), rhs.get(1)) else {
        return Err(invalid(
            "structure",
            "script",
            "assignment right-hand side must be a direct function call",
        ));
    };
    let close = matching_close(rhs, 1)?;
    if close + 1 != rhs.len() {
        return Err(invalid(
            "structure",
            "script",
            "assignment right-hand side must contain only one function call",
        ));
    }
    Ok(Assignment { targets, rhs })
}

fn parse_targets(tokens: &[Token]) -> Result<Vec<String>, AnalysisScriptViolation> {
    let values = match tokens {
        [Token::Ident(name)] => vec![name.clone()],
        [Token::LBracket, middle @ .., Token::RBracket] => {
            if middle.is_empty() {
                return Err(invalid("assignment", "alias", "target list is empty"));
            }
            let mut values = Vec::new();
            for (index, token) in middle.iter().enumerate() {
                if index % 2 == 0 {
                    let Token::Ident(name) = token else {
                        return Err(invalid("assignment", "alias", "invalid target list"));
                    };
                    values.push(name.clone());
                } else if token != &Token::Comma {
                    return Err(invalid("assignment", "alias", "invalid target list"));
                }
            }
            values
        }
        _ => return Err(invalid("assignment", "alias", "invalid assignment target")),
    };
    if values.iter().any(|name| name == "_") {
        return Err(invalid(
            "assignment",
            "alias",
            "discard targets are not allowed in analysis scripts",
        ));
    }
    Ok(values)
}

#[derive(Clone, Debug)]
enum Arity {
    Exact(usize),
    Minimum(usize),
    Range { minimum: usize, maximum: usize },
    Allowed(Vec<usize>),
}

impl Arity {
    fn accepts(&self, actual: usize) -> bool {
        match self {
            Self::Exact(expected) => actual == *expected,
            Self::Minimum(minimum) => actual >= *minimum,
            Self::Range { minimum, maximum } => (*minimum..=*maximum).contains(&actual),
            Self::Allowed(values) => values.contains(&actual),
        }
    }
}

struct FunctionCatalog {
    arities: HashMap<String, Arity>,
    names: HashSet<String>,
}

impl FunctionCatalog {
    fn parse(functions: &Value) -> Result<Self, AnalysisScriptViolation> {
        let mut arities = HashMap::new();
        for entry in functions
            .get("frame_functions")
            .and_then(Value::as_array)
            .ok_or_else(|| invalid("catalog", "functions", "frame_functions is missing"))?
        {
            let name = entry
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| invalid("catalog", "functions", "function name is missing"))?;
            let arity = parse_arity(entry.get("arity")).ok_or_else(|| {
                invalid(
                    "catalog",
                    "functions",
                    format!("arity is invalid for {name}"),
                )
            })?;
            insert_with_aliases(&mut arities, entry, name, arity);
        }
        let fixed = functions
            .get("expression_functions_by_arity")
            .and_then(Value::as_object)
            .ok_or_else(|| invalid("catalog", "functions", "expression arities are missing"))?;
        for (arity, names) in fixed {
            let arity = arity
                .parse::<usize>()
                .map_err(|_| invalid("catalog", "functions", "expression arity is invalid"))?;
            for name in names
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
            {
                arities.insert(name.to_ascii_lowercase(), Arity::Exact(arity));
            }
        }
        for entry in functions
            .get("variable_arity_expression_functions")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                continue;
            };
            let arity = if let Some(values) = entry.get("allowed_arities").and_then(Value::as_array)
            {
                Arity::Allowed(
                    values
                        .iter()
                        .filter_map(Value::as_u64)
                        .map(|value| value as usize)
                        .collect(),
                )
            } else {
                let minimum = entry.get("min_arity").and_then(Value::as_u64).unwrap_or(0) as usize;
                match entry.get("max_arity").and_then(Value::as_u64) {
                    Some(maximum) => Arity::Range {
                        minimum,
                        maximum: maximum as usize,
                    },
                    None => Arity::Minimum(minimum),
                }
            };
            insert_with_aliases(&mut arities, entry, name, arity);
        }
        for entry in functions
            .get("special_expression_forms")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let Some(name) = entry.get("name").and_then(Value::as_str) else {
                continue;
            };
            insert_with_aliases(&mut arities, entry, name, Arity::Minimum(1));
        }
        let names = arities.keys().cloned().collect();
        Ok(Self { arities, names })
    }
}

fn insert_with_aliases(
    target: &mut HashMap<String, Arity>,
    entry: &Value,
    name: &str,
    arity: Arity,
) {
    target.insert(name.to_ascii_lowercase(), arity.clone());
    for alias in entry
        .get("aliases")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        target.insert(alias.to_ascii_lowercase(), arity.clone());
    }
}

fn parse_arity(value: Option<&Value>) -> Option<Arity> {
    match value? {
        Value::Number(value) => Some(Arity::Exact(value.as_u64()? as usize)),
        Value::String(value) => value
            .strip_prefix(">=")
            .and_then(|value| value.parse().ok())
            .map(Arity::Minimum),
        _ => None,
    }
}

fn validate_calls(
    tokens: &[Token],
    catalog: &FunctionCatalog,
) -> Result<(), AnalysisScriptViolation> {
    for index in 0..tokens.len().saturating_sub(1) {
        let (Token::Ident(name), Token::LParen) = (&tokens[index], &tokens[index + 1]) else {
            continue;
        };
        let normalized = name.to_ascii_lowercase();
        if forbidden_function(&normalized) {
            return Err(AnalysisScriptViolation {
                code: "analysis_function_forbidden",
                fields: vec!["script".into(), "function".into()],
                diagnostic: json!({"stage":"functions","function":name}),
            });
        }
        let close = matching_close(tokens, index + 1)?;
        let actual = argument_count(&tokens[index + 2..close])?;
        let Some(arity) = catalog.arities.get(&normalized) else {
            return Err(invalid(
                "functions",
                "function",
                format!("function '{name}' is not present in the current catalog"),
            ));
        };
        if !arity.accepts(actual) {
            return Err(AnalysisScriptViolation {
                code: "analysis_script_invalid",
                fields: vec!["script".into(), "function".into()],
                diagnostic: json!({
                    "stage":"arity",
                    "function":name,
                    "actual":actual,
                }),
            });
        }
    }
    Ok(())
}

fn forbidden_function(name: &str) -> bool {
    name.starts_with("import")
        || name.starts_with("read")
        || name.starts_with("project")
        || name.starts_with("ds_")
        || name.starts_with("dss_")
        || name.contains("_dss")
        || name.contains("url")
        || name.contains("network_study")
        || name.contains("power_flow")
        || name.contains("fault_study")
}

fn matching_close(tokens: &[Token], open: usize) -> Result<usize, AnalysisScriptViolation> {
    let mut depth = 0_i32;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token {
            Token::LParen => depth += 1,
            Token::RParen => {
                depth -= 1;
                if depth == 0 {
                    return Ok(index);
                }
            }
            _ => {}
        }
    }
    Err(invalid("structure", "script", "unbalanced function call"))
}

fn argument_count(tokens: &[Token]) -> Result<usize, AnalysisScriptViolation> {
    if tokens.is_empty() {
        return Ok(0);
    }
    let mut depth = 0_i32;
    let mut count = 1;
    for token in tokens {
        match token {
            Token::LParen | Token::LBracket => depth += 1,
            Token::RParen | Token::RBracket => {
                depth -= 1;
                if depth < 0 {
                    return Err(invalid("structure", "script", "unbalanced argument"));
                }
            }
            Token::Comma if depth == 0 => count += 1,
            _ => {}
        }
    }
    if depth != 0 {
        return Err(invalid("structure", "script", "unbalanced argument"));
    }
    Ok(count)
}

fn invalid(
    stage: &'static str,
    field: &'static str,
    message: impl Into<String>,
) -> AnalysisScriptViolation {
    AnalysisScriptViolation {
        code: "analysis_script_invalid",
        fields: vec![field.into()],
        diagnostic: json!({"stage":stage,"message":message.into()}),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog() -> Value {
        serde_json::json!({
            "frame_functions": [
                {"name":"select","arity":">=2","signature":["expr...","frame"]},
                {"name":"join","arity":5,"signature":["left","right","left_columns","right_columns","how"]},
                {"name":"assert_all","arity":2,"signature":["predicate","frame"]},
                {"name":"explode","arity":2,"signature":["columns","frame"]},
                {"name":"import_df_url","arity":1,"signature":["url"]},
                {"name":"ds_power_flow","arity":2,"signature":["bus","para"]}
            ],
            "expression_functions_by_arity": {
                "1": ["col"],
                "2": ["gt", "alias"]
            },
            "variable_arity_expression_functions": [
                {"name":"cols","min_arity":1}
            ],
            "special_expression_forms": []
        })
    }

    fn inspect(source: &str, outputs: &[&str]) -> Result<(), AnalysisScriptViolation> {
        inspect_analysis_script(
            source,
            &["left".into(), "right".into()],
            &outputs
                .iter()
                .map(|value| (*value).to_string())
                .collect::<Vec<_>>(),
            &catalog(),
        )
    }

    #[test]
    fn analysis_script_inspector_accepts_cataloged_multistep_program() {
        let source = "selected = select(col(bus), col(base), col(phases), left);\
                      joined = join(selected, right, bus, bus, left);\
                      guarded = assert_all(gt(col(base), 0), joined);\
                      expanded = explode(phases, guarded);\
                      result = select(col(bus), alias(col(phases), phase), expanded);";

        assert!(inspect(source, &["result"]).is_ok());
    }

    #[test]
    fn analysis_script_inspector_rejects_invalid_program_shapes() {
        let cases = [
            ("result = select(col(bus), left)", "missing semicolon"),
            ("select(col(bus), left);", "bare expression"),
            ("result = left;", "direct alias"),
            ("left = select(col(bus), left);", "input rebinding"),
            ("result = select(col(bus), left);", "wrong terminal output"),
            (
                "result = select(col(bus), left); result = select(col(bus), right);",
                "duplicate target",
            ),
            ("result = mystery(col(bus), left);", "unknown function"),
            ("result = select(left);", "wrong arity"),
        ];

        for (source, label) in cases {
            let outputs = if label == "wrong terminal output" {
                vec!["other"]
            } else {
                vec!["result"]
            };
            let violation = inspect(source, &outputs).expect_err(label);
            assert_eq!(violation.code, "analysis_script_invalid", "{label}");
            assert!(!violation.fields.is_empty(), "{label}");
        }
    }

    #[test]
    fn analysis_script_inspector_rejects_nested_io_and_dsa_calls() {
        for source in [
            "result = select(import_df_url(source), left);",
            "result = select(ds_power_flow(left, right), left);",
        ] {
            let forbidden = inspect(source, &["result"]).expect_err("forbidden function");
            assert_eq!(forbidden.code, "analysis_function_forbidden");
        }
    }
}
