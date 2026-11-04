use sha2::{Digest, Sha256};

const MAX_PRESENTED_TOKEN_BYTES: usize = 256;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TokenError {
    Invalid,
}

pub(crate) fn generate_token() -> String {
    let mut bytes = [0_u8; 32];
    getrandom::fill(&mut bytes).expect("operating system random source unavailable");
    hex::encode(bytes)
}

pub(crate) fn hash_token(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub(crate) fn validate_presented_token(token: &str) -> Result<(), TokenError> {
    if token.is_empty() || token.len() > MAX_PRESENTED_TOKEN_BYTES {
        Err(TokenError::Invalid)
    } else {
        Ok(())
    }
}

pub(crate) fn is_generated_token(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_tokens_are_distinct_lowercase_hex_secrets() {
        let first = generate_token();
        let second = generate_token();
        assert_eq!(first.len(), 64);
        assert!(first.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(first, first.to_ascii_lowercase());
        assert_ne!(first, second);
    }

    #[test]
    fn token_hash_is_stable_and_does_not_expose_plaintext() {
        let token = "secret-preview-token";
        let hash = hash_token(token);
        assert_eq!(hash.len(), 64);
        assert_eq!(hash, hash_token(token));
        assert_ne!(hash, token);
        assert!(!hash.contains(token));
    }

    #[test]
    fn presented_tokens_accept_bounded_legacy_values_only() {
        assert_eq!(
            validate_presented_token(&format!("dr-delete_{}", "a".repeat(64))),
            Ok(())
        );
        assert_eq!(
            validate_presented_token(&format!("dr-replace_{}", "b".repeat(64))),
            Ok(())
        );
        assert_eq!(validate_presented_token(""), Err(TokenError::Invalid));
        assert_eq!(
            validate_presented_token(&"x".repeat(257)),
            Err(TokenError::Invalid)
        );
    }

    #[test]
    fn generated_token_format_rejects_legacy_values() {
        assert!(is_generated_token(&"a".repeat(64)));
        assert!(!is_generated_token(&format!(
            "dr-delete_{}",
            "a".repeat(64)
        )));
        assert!(!is_generated_token(&"g".repeat(64)));
    }
}
