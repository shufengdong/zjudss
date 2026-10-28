#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    Ident(String),
    Literal,
    LBracket,
    RBracket,
    LParen,
    RParen,
    Comma,
    Equal,
    Semicolon,
    Other(char),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssignmentCall {
    pub targets: Vec<String>,
    pub callee: String,
    pub arguments: Vec<String>,
}

pub fn tokenize(source: &str) -> Result<Vec<Token>, String> {
    let chars = source.chars().collect::<Vec<_>>();
    let mut tokens = Vec::new();
    let mut index = 0;
    while index < chars.len() {
        let value = chars[index];
        if value.is_whitespace() {
            index += 1;
            continue;
        }
        if value == '/' && chars.get(index + 1) == Some(&'/') {
            index += 2;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            continue;
        }
        if value == '/' && chars.get(index + 1) == Some(&'*') {
            index += 2;
            let mut closed = false;
            while index + 1 < chars.len() {
                if chars[index] == '*' && chars[index + 1] == '/' {
                    index += 2;
                    closed = true;
                    break;
                }
                index += 1;
            }
            if !closed {
                return Err("未闭合的块注释".into());
            }
            continue;
        }
        if value == '"' || value == '\'' {
            let quote = value;
            index += 1;
            let mut closed = false;
            while index < chars.len() {
                if chars[index] == '\\' {
                    index = (index + 2).min(chars.len());
                    continue;
                }
                if chars[index] == quote {
                    index += 1;
                    closed = true;
                    break;
                }
                index += 1;
            }
            if !closed {
                return Err("未闭合的字符串".into());
            }
            tokens.push(Token::Literal);
            continue;
        }
        if value.is_ascii_alphabetic() || value == '_' {
            let start = index;
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                index += 1;
            }
            tokens.push(Token::Ident(chars[start..index].iter().collect()));
            continue;
        }
        if value.is_ascii_digit()
            || value == '.' && chars.get(index + 1).is_some_and(char::is_ascii_digit)
        {
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric()
                    || matches!(chars[index], '.' | '_' | '+' | '-'))
            {
                index += 1;
            }
            tokens.push(Token::Literal);
            continue;
        }
        tokens.push(match value {
            '[' => Token::LBracket,
            ']' => Token::RBracket,
            '(' => Token::LParen,
            ')' => Token::RParen,
            ',' => Token::Comma,
            '=' => Token::Equal,
            ';' => Token::Semicolon,
            other => Token::Other(other),
        });
        index += 1;
    }
    Ok(tokens)
}

pub fn direct_assignment_calls(tokens: &[Token]) -> Vec<AssignmentCall> {
    split_statements(tokens)
        .into_iter()
        .filter_map(parse_direct_assignment_call)
        .collect()
}

pub fn called_functions(tokens: &[Token]) -> Vec<String> {
    tokens
        .windows(2)
        .filter_map(|pair| match pair {
            [Token::Ident(name), Token::LParen] => Some(name.clone()),
            _ => None,
        })
        .collect()
}

pub fn assigned_identifiers(tokens: &[Token]) -> Vec<String> {
    let mut names = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if token != &Token::Equal {
            continue;
        }
        match index.checked_sub(1).and_then(|index| tokens.get(index)) {
            Some(Token::Ident(name)) => names.push(name.clone()),
            Some(Token::RBracket) => {
                let mut depth = 0_i32;
                for start in (0..index).rev() {
                    match &tokens[start] {
                        Token::RBracket => depth += 1,
                        Token::LBracket => {
                            depth -= 1;
                            if depth == 0 {
                                if let Some(identifiers) = parse_ident_list(&tokens[start..index]) {
                                    names.extend(identifiers);
                                }
                                break;
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    names
}

fn split_statements(tokens: &[Token]) -> Vec<&[Token]> {
    let mut statements = Vec::new();
    let mut start = 0;
    let mut depth = 0_i32;
    for (index, token) in tokens.iter().enumerate() {
        match token {
            Token::LParen | Token::LBracket => depth += 1,
            Token::RParen | Token::RBracket => depth -= 1,
            Token::Semicolon if depth == 0 => {
                if start < index {
                    statements.push(&tokens[start..index]);
                }
                start = index + 1;
            }
            _ => {}
        }
    }
    if start < tokens.len() {
        statements.push(&tokens[start..]);
    }
    statements
}

fn parse_direct_assignment_call(statement: &[Token]) -> Option<AssignmentCall> {
    let equal = statement.iter().position(|token| token == &Token::Equal)?;
    let targets = parse_ident_list(&statement[..equal])?;
    let rhs = &statement[equal + 1..];
    let (Token::Ident(callee), Some(Token::LParen), Some(Token::RParen)) =
        (rhs.first()?, rhs.get(1), rhs.last())
    else {
        return None;
    };
    let arguments = parse_ident_list(&rhs[2..rhs.len() - 1])?;
    Some(AssignmentCall {
        targets,
        callee: callee.clone(),
        arguments,
    })
}

fn parse_ident_list(tokens: &[Token]) -> Option<Vec<String>> {
    let tokens = match (tokens.first(), tokens.last()) {
        (Some(Token::LBracket), Some(Token::RBracket)) => &tokens[1..tokens.len() - 1],
        _ => tokens,
    };
    if tokens.is_empty() {
        return Some(Vec::new());
    }
    let mut names = Vec::new();
    for (index, token) in tokens.iter().enumerate() {
        if index % 2 == 0 {
            let Token::Ident(name) = token else {
                return None;
            };
            names.push(name.clone());
        } else if token != &Token::Comma {
            return None;
        }
    }
    Some(names)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_direct_destructuring_call_and_ignores_comments() {
        let tokens = tokenize("// import_dss(\"x\")\n[a,b] = solve(table, para); a;").unwrap();
        assert_eq!(called_functions(&tokens), vec!["solve"]);
        assert_eq!(
            direct_assignment_calls(&tokens),
            vec![AssignmentCall {
                targets: vec!["a".into(), "b".into()],
                callee: "solve".into(),
                arguments: vec!["table".into(), "para".into()],
            }]
        );
    }

    #[test]
    fn refuses_nested_or_computed_arguments() {
        let tokens = tokenize("result = solve(select(table), para);").unwrap();
        assert!(direct_assignment_calls(&tokens).is_empty());
        assert_eq!(called_functions(&tokens), vec!["solve", "select"]);
    }

    #[test]
    fn extracts_all_direct_assignment_targets() {
        let tokens =
            tokenize("if (ok) { table = select(table); } [a,b] = solve(table, para);").unwrap();
        assert_eq!(assigned_identifiers(&tokens), vec!["table", "a", "b"]);
    }
}
