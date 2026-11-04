use super::uploads::CandidatePurpose;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CandidatePolicyError {
    Immutable,
    ConfirmationInProgress,
    AliasRequired,
    ReplacementActionRequired,
}

pub(crate) fn validate_mutable_state(
    status: &str,
    confirmation_in_progress: bool,
) -> Result<(), CandidatePolicyError> {
    if confirmation_in_progress {
        Err(CandidatePolicyError::ConfirmationInProgress)
    } else if matches!(status, "failed" | "discarded" | "confirmed" | "expired") {
        Err(CandidatePolicyError::Immutable)
    } else {
        Ok(())
    }
}

pub(crate) fn validate_selected_alias(
    selected: bool,
    purpose: CandidatePurpose,
    alias: Option<&str>,
) -> Result<(), CandidatePolicyError> {
    if selected
        && purpose != CandidatePurpose::Skip
        && alias.is_none_or(|value| value.trim().is_empty())
    {
        Err(CandidatePolicyError::AliasRequired)
    } else {
        Ok(())
    }
}

pub(crate) fn selected_for_storage(selected: bool, purpose: CandidatePurpose) -> bool {
    selected && purpose != CandidatePurpose::Skip
}

pub(crate) fn validate_replacement_action(
    has_replacement_target: bool,
    has_action: bool,
) -> Result<(), CandidatePolicyError> {
    if has_replacement_target && !has_action {
        Err(CandidatePolicyError::ReplacementActionRequired)
    } else {
        Ok(())
    }
}

pub(crate) fn settled_status(model_status: &str, resource_status: &str) -> &'static str {
    let model_done = matches!(model_status, "published" | "not_applicable");
    let resource_done = matches!(resource_status, "published" | "not_applicable");
    if model_done && resource_done {
        "confirmed"
    } else if model_status == "published" || resource_status == "published" {
        "partially_confirmed"
    } else {
        "ready_to_confirm"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::CandidatePurpose;

    #[test]
    fn only_live_unlocked_batch_states_are_mutable() {
        for status in [
            "processing",
            "awaiting_parse_settings",
            "awaiting_purpose",
            "ready_to_confirm",
            "partially_confirmed",
        ] {
            assert_eq!(validate_mutable_state(status, false), Ok(()), "{status}");
        }
        for status in ["failed", "discarded", "confirmed", "expired"] {
            assert_eq!(
                validate_mutable_state(status, false),
                Err(CandidatePolicyError::Immutable),
                "{status}"
            );
        }
        assert_eq!(
            validate_mutable_state("ready_to_confirm", true),
            Err(CandidatePolicyError::ConfirmationInProgress)
        );
    }

    #[test]
    fn selected_candidates_require_an_alias_and_skip_is_never_stored_selected() {
        assert_eq!(
            validate_selected_alias(true, CandidatePurpose::DataResource, None),
            Err(CandidatePolicyError::AliasRequired)
        );
        assert_eq!(
            validate_selected_alias(true, CandidatePurpose::ModelTable, Some("table")),
            Ok(())
        );
        assert!(!selected_for_storage(true, CandidatePurpose::Skip));
        assert!(!selected_for_storage(false, CandidatePurpose::DataResource));
        assert!(selected_for_storage(true, CandidatePurpose::DataResource));
    }

    #[test]
    fn replacement_batches_require_an_explicit_action() {
        assert_eq!(validate_replacement_action(false, false), Ok(()));
        assert_eq!(validate_replacement_action(false, true), Ok(()));
        assert_eq!(validate_replacement_action(true, true), Ok(()));
        assert_eq!(
            validate_replacement_action(true, false),
            Err(CandidatePolicyError::ReplacementActionRequired)
        );
    }

    #[test]
    fn child_statuses_have_one_final_batch_status_table() {
        assert_eq!(settled_status("published", "published"), "confirmed");
        assert_eq!(settled_status("not_applicable", "published"), "confirmed");
        assert_eq!(settled_status("published", "failed"), "partially_confirmed");
        assert_eq!(settled_status("failed", "published"), "partially_confirmed");
        assert_eq!(settled_status("failed", "failed"), "ready_to_confirm");
        assert_eq!(settled_status("ready", "ready"), "ready_to_confirm");
    }
}
