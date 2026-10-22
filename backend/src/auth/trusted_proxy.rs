use axum::http::HeaderMap;

use super::{AccessScope, AuthError, Principal, StorageScopeRef};

const SUBJECT_HEADER: &str = "x-zjudss-subject";
const TENANT_HEADER: &str = "x-zjudss-tenant";
const TOKEN_HEADER: &str = "x-zjudss-proxy-token";

pub fn authenticate(headers: &HeaderMap, expected_token: &str) -> Result<Principal, AuthError> {
    let supplied_token = header(headers, TOKEN_HEADER)?;
    if !constant_time_eq(supplied_token.as_bytes(), expected_token.as_bytes()) {
        return Err(AuthError::Unauthorized);
    }

    let subject = header(headers, SUBJECT_HEADER)?;
    validate_identity(subject, 256)?;
    let tenant = headers
        .get(TENANT_HEADER)
        .map(|value| value.to_str().map_err(|_| AuthError::Unauthorized))
        .transpose()?;
    if let Some(tenant) = tenant {
        validate_identity(tenant, 128)?;
    }

    let access_key = match tenant {
        Some(tenant) => format!("tenant:{tenant}:owner:{subject}"),
        None => format!("owner:{subject}"),
    };
    Ok(Principal {
        access: AccessScope {
            kind: "owner".to_string(),
            key: access_key.clone(),
        },
        storage: StorageScopeRef {
            kind: "owner".to_string(),
            key: access_key,
        },
        subject: subject.to_string(),
    })
}

fn header<'a>(headers: &'a HeaderMap, name: &str) -> Result<&'a str, AuthError> {
    headers
        .get(name)
        .ok_or(AuthError::Unauthorized)?
        .to_str()
        .map_err(|_| AuthError::Unauthorized)
}

fn validate_identity(value: &str, max_len: usize) -> Result<(), AuthError> {
    if value.is_empty() || value.len() > max_len || value.chars().any(char::is_control) {
        return Err(AuthError::Unauthorized);
    }
    Ok(())
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    let max_len = left.len().max(right.len());
    for index in 0..max_len {
        let l = left.get(index).copied().unwrap_or(0);
        let r = right.get(index).copied().unwrap_or(0);
        difference |= usize::from(l ^ r);
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::constant_time_eq;

    #[test]
    fn token_comparison_checks_length_and_contents() {
        assert!(constant_time_eq(b"secret", b"secret"));
        assert!(!constant_time_eq(b"secret", b"secrex"));
        assert!(!constant_time_eq(b"secret", b"secret-longer"));
    }
}
