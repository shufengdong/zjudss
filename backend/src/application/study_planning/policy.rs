use crate::engine_resources::{
    FacadeContract, FacadeInputKind, FacadeRegistry, ResultSelectionRule,
};

use super::contracts::*;

/// Legacy `plan_study` adapter. Facade discovery, arity, inputs and results all
/// come from the current immutable engine snapshot. V4 managed studies use the
/// same registry directly when building their confirmation configuration.
pub fn evaluate(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    facades: &FacadeRegistry,
) -> StudyPlan {
    if facts.active_version_id.is_none() {
        return plan(
            request,
            facts,
            StudyStatus::MissingModelData,
            None,
            Vec::new(),
            vec![missing(
                "model.active",
                "当前会话没有 active 模型",
                Vec::new(),
            )],
            Vec::new(),
        );
    }

    let candidates = family_candidates(request, facades);
    if candidates.is_empty() {
        return plan(
            request,
            facts,
            StudyStatus::Unsupported,
            None,
            Vec::new(),
            Vec::new(),
            vec!["当前引擎 snapshot 没有与该研究意图匹配的可用门面".into()],
        );
    }
    let selected = match select_candidate(request, facts, &candidates) {
        Some(selected) => selected,
        None => {
            return plan(
                request,
                facts,
                StudyStatus::NeedsClarification,
                None,
                Vec::new(),
                vec![missing(
                    "facadeId",
                    "多个研究门面都符合当前意图，请明确选择",
                    candidates.iter().map(|facade| facade.id.clone()).collect(),
                )],
                candidates
                    .iter()
                    .flat_map(|facade| facade.differences.clone())
                    .collect(),
            );
        }
    };

    let model_input_order = selected
        .inputs
        .iter()
        .filter(|input| input.kind == FacadeInputKind::Model)
        .map(|input| input.alias.clone())
        .collect::<Vec<_>>();
    let missing_model = model_input_order
        .iter()
        .filter(|alias| !facts.available_aliases.contains(alias.as_str()))
        .map(|alias| {
            missing(
                &format!("model.tables.{alias}"),
                "当前 active 模型缺少该门面声明的模型输入",
                Vec::new(),
            )
        })
        .collect::<Vec<_>>();
    if !missing_model.is_empty() {
        return plan(
            request,
            facts,
            StudyStatus::MissingModelData,
            Some(selected),
            Vec::new(),
            missing_model,
            Vec::new(),
        );
    }

    let requested = requested_results(request);
    let results = if requested.is_empty() {
        default_results(selected)
    } else {
        requested.to_vec()
    };
    let available_results = selected
        .results
        .iter()
        .map(|result| result.name.as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let unknown = results
        .iter()
        .filter(|result| !available_results.contains(result.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    if !unknown.is_empty() {
        return plan(
            request,
            facts,
            StudyStatus::NeedsClarification,
            Some(selected),
            Vec::new(),
            vec![missing(
                "requestedResults",
                "请求包含当前门面未声明的结果",
                selected
                    .results
                    .iter()
                    .map(|result| result.name.clone())
                    .collect(),
            )],
            vec![format!("未知结果: {}", unknown.join(", "))],
        );
    }

    let input_aliases = facts.available_aliases.iter().cloned().collect::<Vec<_>>();
    let solver = resolved_solver(selected);
    let ready = ReadyStudyPlan {
        model_version_id: facts
            .active_version_id
            .expect("active model checked before ready plan"),
        model_content_hash: facts.content_hash.clone().unwrap_or_default(),
        engine_snapshot_id: facts.engine_snapshot_id.clone(),
        input_aliases: input_aliases.clone(),
        model_input_order: model_input_order.clone(),
        request: request.clone(),
        function: selected.function.clone(),
        arity: selected.inputs.len(),
        results: results.clone(),
        solver,
    };
    StudyPlan {
        status: StudyStatus::Ready,
        study_type: request.kind(),
        model_facts: facts.into(),
        input_aliases,
        model_input_order,
        function: Some(selected.function.clone()),
        arity: Some(selected.inputs.len()),
        solver,
        results,
        missing_parameters: Vec::new(),
        warnings: Vec::new(),
        ready: Some(ready),
    }
}

fn family_candidates<'a>(
    request: &StudyRequest,
    facades: &'a FacadeRegistry,
) -> Vec<&'a FacadeContract> {
    let family = match request {
        StudyRequest::StaticPowerFlow { .. } => "static_power_flow",
        StudyRequest::Qsts { .. } => "qsts",
        StudyRequest::Fault { .. } => "short_circuit",
        StudyRequest::Harmonic { .. } => "harmonics",
        StudyRequest::Transient { subtype, .. } => {
            if !matches!(subtype, Some(TransientSubtype::MotorDynamics)) {
                return Vec::new();
            }
            "dynamics"
        }
    };
    facades
        .enabled()
        .filter(|facade| facade.family == family)
        .filter(|facade| {
            !matches!(request, StudyRequest::Transient { .. }) || facade.id == "motor_dynamics"
        })
        .collect()
}

fn select_candidate<'a>(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    candidates: &[&'a FacadeContract],
) -> Option<&'a FacadeContract> {
    if candidates.len() == 1 {
        return candidates.first().copied();
    }
    let preference = match request {
        StudyRequest::StaticPowerFlow {
            solver_preference, ..
        }
        | StudyRequest::Qsts {
            solver_preference, ..
        } => Some(*solver_preference),
        _ => None,
    }?;
    let preference = match preference {
        SolverPreference::Radial => "radial",
        SolverPreference::Newton => "newton",
        SolverPreference::Auto => {
            if facts.voltage_source_count == 1
                && facts.is_connected
                && !facts.has_loops
                && !facts.has_parallel_branches
                && !facts.has_pv_generator
                && !facts.has_gfm
            {
                "radial"
            } else {
                "newton"
            }
        }
    };
    candidates
        .iter()
        .copied()
        .find(|facade| facade.id.to_ascii_lowercase().contains(preference))
}

fn default_results(facade: &FacadeContract) -> Vec<String> {
    facade
        .result_constraints
        .iter()
        .find_map(|constraint| match constraint {
            ResultSelectionRule::DefaultResults { results }
            | ResultSelectionRule::FixedResults { results, .. } => Some(results.clone()),
            _ => None,
        })
        .unwrap_or_else(|| {
            facade
                .results
                .iter()
                .filter(|result| result.recommended)
                .map(|result| result.name.clone())
                .collect()
        })
}

fn requested_results(request: &StudyRequest) -> &[String] {
    match request {
        StudyRequest::StaticPowerFlow {
            requested_results, ..
        }
        | StudyRequest::Qsts {
            requested_results, ..
        }
        | StudyRequest::Fault {
            requested_results, ..
        }
        | StudyRequest::Harmonic {
            requested_results, ..
        }
        | StudyRequest::Transient {
            requested_results, ..
        } => requested_results,
    }
}

fn resolved_solver(facade: &FacadeContract) -> Option<ResolvedSolver> {
    let id = facade.id.to_ascii_lowercase();
    if id.contains("newton") {
        Some(ResolvedSolver::Newton)
    } else if id.contains("radial") {
        Some(ResolvedSolver::Radial)
    } else {
        None
    }
}

fn missing(field: &str, reason: &str, choices: Vec<String>) -> MissingParameter {
    MissingParameter {
        field: field.to_string(),
        reason: reason.to_string(),
        choices,
    }
}

fn plan(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    status: StudyStatus,
    facade: Option<&FacadeContract>,
    results: Vec<String>,
    missing_parameters: Vec<MissingParameter>,
    warnings: Vec<String>,
) -> StudyPlan {
    let input_aliases = facts.available_aliases.iter().cloned().collect::<Vec<_>>();
    let model_input_order = facade
        .map(|facade| {
            facade
                .inputs
                .iter()
                .filter(|input| input.kind == FacadeInputKind::Model)
                .map(|input| input.alias.clone())
                .collect()
        })
        .unwrap_or_default();
    StudyPlan {
        status,
        study_type: request.kind(),
        model_facts: facts.into(),
        input_aliases,
        model_input_order,
        function: facade.map(|facade| facade.function.clone()),
        arity: facade.map(|facade| facade.inputs.len()),
        solver: facade.and_then(resolved_solver),
        results,
        missing_parameters,
        warnings,
        ready: None,
    }
}
