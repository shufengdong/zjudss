use crate::engine_resources::{EngineResourceSnapshot, FacadeInputKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AliasPurpose {
    ModelTable,
    DataResource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AliasError {
    Invalid,
    Reserved,
}

pub(crate) struct ResourceAliasPolicy<'a> {
    snapshot: &'a EngineResourceSnapshot,
}

impl<'a> ResourceAliasPolicy<'a> {
    pub(crate) fn new(snapshot: &'a EngineResourceSnapshot) -> Self {
        Self { snapshot }
    }

    pub(crate) fn validate(
        &self,
        alias: &str,
        purpose: AliasPurpose,
        schema_ref: Option<&str>,
    ) -> Result<(), AliasError> {
        let alias = alias.trim();
        if !valid_identifier(alias) {
            return Err(AliasError::Invalid);
        }
        if purpose == AliasPurpose::ModelTable {
            return Ok(());
        }
        let allowed_input = self
            .snapshot
            .facades
            .enabled()
            .flat_map(|facade| &facade.inputs)
            .any(|input| {
                input.alias.eq_ignore_ascii_case(alias)
                    && matches!(input.kind, FacadeInputKind::Data | FacadeInputKind::Struct)
                    && schema_ref.is_none_or(|schema| schema == input.schema_ref)
            });
        if allowed_input {
            return Ok(());
        }
        let reserved_model = self
            .snapshot
            .facades
            .enabled()
            .flat_map(|facade| &facade.inputs)
            .any(|input| {
                input.alias.eq_ignore_ascii_case(alias) && input.kind == FacadeInputKind::Model
            });
        let reserved_output = self.snapshot.facades.enabled().any(|facade| {
            facade.function.eq_ignore_ascii_case(alias)
                || facade
                    .results
                    .iter()
                    .any(|result| result.name.eq_ignore_ascii_case(alias))
        });
        if reserved_model
            || reserved_output
            || alias.starts_with("__")
            || ["tmp", "result", "output"].contains(&alias.to_ascii_lowercase().as_str())
        {
            Err(AliasError::Reserved)
        } else {
            Ok(())
        }
    }
}

pub(crate) fn normalize_alias(value: &str) -> String {
    value.trim().to_ascii_lowercase().replace([' ', '-'], "_")
}

pub(crate) fn default_alias(value: &str) -> String {
    let stem = std::path::Path::new(value)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or(value);
    let value = normalize_alias(stem);
    if valid_identifier(&value) {
        value
    } else {
        "data".into()
    }
}

pub(crate) fn valid_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    matches!(chars.next(),Some(first) if first=='_'||first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use crate::engine_resources::EngineResourceSnapshot;

    use super::*;

    #[test]
    fn normalization_preserves_the_existing_backend_contract() {
        assert_eq!(normalize_alias("  Load-Profile A  "), "load_profile_a");
        assert_eq!(normalize_alias("already_valid"), "already_valid");
    }

    #[test]
    fn default_alias_uses_the_stem_and_falls_back_for_invalid_identifiers() {
        assert_eq!(default_alias("Load-Profile.csv"), "load_profile");
        assert_eq!(default_alias("123.csv"), "data");
        assert_eq!(default_alias("负荷.csv"), "data");
    }

    #[test]
    fn rss_identifiers_are_ascii_and_have_a_valid_first_character() {
        assert!(valid_identifier("_load_1"));
        assert!(valid_identifier("Load1"));
        assert!(!valid_identifier("1load"));
        assert!(!valid_identifier("load-profile"));
        assert!(!valid_identifier("负荷"));
    }

    #[test]
    fn snapshot_policy_allows_matching_inputs_and_rejects_reserved_names() {
        let snapshot = EngineResourceSnapshot::minimal("test", "v1", PathBuf::from("."));
        let policy = ResourceAliasPolicy::new(&snapshot);

        assert_eq!(
            policy.validate("para", AliasPurpose::DataResource, Some("frames.para")),
            Ok(())
        );
        assert_eq!(
            policy.validate("table", AliasPurpose::ModelTable, Some("frames.table")),
            Ok(())
        );
        for alias in ["table", "test", "result", "__internal", "tmp", "output"] {
            assert_eq!(
                policy.validate(alias, AliasPurpose::DataResource, None),
                Err(AliasError::Reserved),
                "{alias} must remain reserved"
            );
        }
        assert_eq!(
            policy.validate("bad-name", AliasPurpose::DataResource, None),
            Err(AliasError::Invalid)
        );
    }
}
