use std::collections::BTreeSet;

use crate::adapters::STATIC_MODEL_TABLES;

use super::contracts::*;

const MAX_ITEMS: usize = 64;
const MAX_PROFILE_POINTS: usize = 1_000;
const MAX_IDENTIFIER_BYTES: usize = 256;

pub fn evaluate(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    functions: &StudyFunctionCatalog,
) -> StudyPlan {
    if let StudyRequest::Transient {
        subtype: Some(subtype),
        ..
    } = request
        && *subtype != TransientSubtype::MotorDynamics
    {
        return plan(
            request,
            facts,
            StudyStatus::Unsupported,
            None,
            None,
            vec![],
            vec![],
            vec!["当前引擎只支持电机—网络动态，不支持所请求的暂态类型".into()],
        );
    }
    if facts.active_version_id.is_none() {
        return plan(
            request,
            facts,
            StudyStatus::MissingModelData,
            None,
            None,
            vec![],
            vec![missing("model.active", "当前会话没有 active 模型")],
            vec![],
        );
    }
    let missing_aliases = STATIC_MODEL_TABLES
        .into_iter()
        .filter(|alias| !facts.available_aliases.contains(*alias))
        .map(|alias| {
            missing(
                &format!("model.tables.{alias}"),
                "当前 active 模型缺少该研究输入表",
            )
        })
        .collect::<Vec<_>>();
    if !missing_aliases.is_empty() {
        return plan(
            request,
            facts,
            StudyStatus::MissingModelData,
            None,
            None,
            vec![],
            missing_aliases,
            vec![],
        );
    }
    match request {
        StudyRequest::StaticPowerFlow {
            solver_preference,
            requested_results,
        } => power_flow(
            request,
            facts,
            functions,
            *solver_preference,
            requested_results,
            false,
        ),
        StudyRequest::Qsts {
            solver_preference,
            mode,
            start_seconds,
            step_size_seconds,
            number,
            profile,
            requested_results,
        } => qsts(
            request,
            facts,
            functions,
            *solver_preference,
            mode,
            *start_seconds,
            *step_size_seconds,
            *number,
            profile,
            requested_results,
        ),
        StudyRequest::Fault {
            cases,
            allow_ideal_sources,
            requested_results,
        } => fault(
            request,
            facts,
            functions,
            cases,
            *allow_ideal_sources,
            requested_results,
        ),
        StudyRequest::Harmonic {
            h_max,
            monitor_elements,
            requested_results,
        } => harmonic(
            request,
            facts,
            functions,
            *h_max,
            monitor_elements,
            requested_results,
        ),
        StudyRequest::Transient {
            subtype,
            motor_dynamics,
            requested_results,
        } => transient(
            request,
            facts,
            functions,
            *subtype,
            motor_dynamics,
            requested_results,
        ),
    }
}

fn power_flow(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    functions: &StudyFunctionCatalog,
    preference: SolverPreference,
    requested_results: &[String],
    qsts: bool,
) -> StudyPlan {
    let mut missing_parameters = base_model_missing(facts);
    let inferred = inferred_solver(facts);
    if preference == SolverPreference::Radial && inferred == ResolvedSolver::Newton {
        missing_parameters.push(MissingParameter {
            field: "solverPreference".into(),
            reason: "当前模型包含环网、并联支路、多电源、PV 或 GFM，不能使用 Radial".into(),
            choices: vec!["newton".into()],
        });
    }
    if !missing_parameters.is_empty() {
        let status = if missing_parameters
            .iter()
            .any(|item| item.field.starts_with("model."))
        {
            StudyStatus::MissingModelData
        } else {
            StudyStatus::NeedsClarification
        };
        return plan(
            request,
            facts,
            status,
            None,
            None,
            vec![],
            missing_parameters,
            vec![],
        );
    }
    let solver = match preference {
        SolverPreference::Auto => inferred,
        SolverPreference::Radial => ResolvedSolver::Radial,
        SolverPreference::Newton => ResolvedSolver::Newton,
    };
    let function_name = match (qsts, solver) {
        (false, ResolvedSolver::Radial) => "ds_radial_pf",
        (false, ResolvedSolver::Newton) => "ds_newton_raphson_pf",
        (true, ResolvedSolver::Radial) => "ds_radial_qsts",
        (true, ResolvedSolver::Newton) => "ds_newton_raphson_qsts",
    };
    finish_ready(
        request,
        facts,
        functions,
        function_name,
        Some(solver),
        requested_results,
        if qsts { &[] } else { &["bus_phase", "summary"] },
        if qsts { 33 } else { 21 },
        vec![],
    )
}

#[allow(clippy::too_many_arguments)]
fn qsts(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    functions: &StudyFunctionCatalog,
    preference: SolverPreference,
    mode: &Option<QstsMode>,
    start_seconds: Option<f64>,
    step_size_seconds: Option<f64>,
    number: Option<u32>,
    profile: &Option<QstsProfile>,
    requested_results: &[String],
) -> StudyPlan {
    let mut missing_parameters = Vec::new();
    if mode.is_none() {
        missing_parameters.push(missing("mode", "时序模式会决定曲线和求解语义"));
    }
    if !start_seconds.is_some_and(|value| value.is_finite() && value >= 0.0) {
        missing_parameters.push(missing("startSeconds", "需要有限且非负的起始时间"));
    }
    if !step_size_seconds.is_some_and(|value| value.is_finite() && value > 0.0) {
        missing_parameters.push(missing("stepSizeSeconds", "需要有限且大于零的步长"));
    }
    if !number.is_some_and(|value| (1..=100_000).contains(&value)) {
        missing_parameters.push(missing("number", "求解步数必须在 1..=100000"));
    }
    if profile.as_ref().is_none_or(|value| !valid_profile(value)) {
        missing_parameters.push(missing("profile", "必须明确常数倍率或内联全局曲线"));
    }
    if !missing_parameters.is_empty() {
        return plan(
            request,
            facts,
            StudyStatus::NeedsClarification,
            None,
            None,
            vec![],
            missing_parameters,
            vec![],
        );
    }
    power_flow(
        request,
        facts,
        functions,
        preference,
        requested_results,
        true,
    )
}

fn fault(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    functions: &StudyFunctionCatalog,
    cases: &Option<Vec<FaultCase>>,
    allow_ideal_sources: Option<bool>,
    requested_results: &[String],
) -> StudyPlan {
    let mut missing_parameters = base_model_missing(facts);
    match cases {
        None => missing_parameters.push(missing("cases", "至少需要一个故障 case")),
        Some(cases) if cases.is_empty() => {
            missing_parameters.push(missing("cases", "至少需要一个故障 case"));
        }
        Some(cases) if cases.len() > MAX_ITEMS => {
            missing_parameters.push(missing("cases", "故障 case 最多 64 个"));
        }
        Some(cases) => {
            for (index, case) in cases.iter().enumerate() {
                let bus_valid = case
                    .bus
                    .as_ref()
                    .is_some_and(|bus| valid_identifier(bus) && facts.buses.contains_key(bus));
                if !bus_valid {
                    missing_parameters.push(missing(
                        &format!("cases[{index}].bus"),
                        "故障母线必须存在于当前模型",
                    ));
                }
                let terminals_valid = case.terminal_map.as_ref().is_some_and(|terminals| {
                    !terminals.is_empty()
                        && terminals.len() <= MAX_ITEMS
                        && case.bus.as_ref().is_some_and(|bus| {
                            facts.buses.get(bus).is_some_and(|available| {
                                terminals
                                    .iter()
                                    .all(|terminal| available.contains(terminal))
                            })
                        })
                });
                if !terminals_valid {
                    missing_parameters.push(missing(
                        &format!("cases[{index}].terminalMap"),
                        "故障端子必须属于所选母线",
                    ));
                }
                if case.kind.is_none() {
                    missing_parameters.push(MissingParameter {
                        field: format!("cases[{index}].kind"),
                        reason: "故障类型会改变故障网络".into(),
                        choices: ["slg", "ll", "llg", "2pg", "3p", "3pg"]
                            .into_iter()
                            .map(str::to_string)
                            .collect(),
                    });
                }
                let impedance_valid = case.bolted == Some(true)
                    || (case.bolted == Some(false)
                        && case.zf.is_some_and(f64::is_finite)
                        && case.xf.is_some_and(f64::is_finite));
                if !impedance_valid {
                    missing_parameters.push(missing(
                        &format!("cases[{index}].impedance"),
                        "必须明确金属性短路或提供有限故障阻抗",
                    ));
                }
            }
        }
    }
    let ideal_sources =
        facts.fault_source_equivalent_count < facts.voltage_source_count + facts.generator_count;
    if ideal_sources && allow_ideal_sources.is_none() {
        missing_parameters.push(missing(
            "allowIdealSources",
            "模型包含未声明短路等值的理想电压源，需确认是否按理想源语义计算",
        ));
    }
    if ideal_sources && allow_ideal_sources == Some(false) {
        missing_parameters.push(missing(
            "model.faultSourceEquivalent",
            "请先为理想源补充短路容量或序阻抗",
        ));
    }
    if !missing_parameters.is_empty() {
        let status = if missing_parameters
            .iter()
            .any(|value| value.field.starts_with("model."))
        {
            StudyStatus::MissingModelData
        } else {
            StudyStatus::NeedsClarification
        };
        return plan(
            request,
            facts,
            status,
            Some("ds_fault_study"),
            Some(22),
            vec![],
            missing_parameters,
            vec![],
        );
    }
    finish_ready(
        request,
        facts,
        functions,
        "ds_fault_study",
        None,
        requested_results,
        &[],
        22,
        if ideal_sources {
            vec!["故障结果采用当前模型的理想电压源语义".into()]
        } else {
            vec![]
        },
    )
}

fn harmonic(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    functions: &StudyFunctionCatalog,
    h_max: Option<u32>,
    monitor_elements: &[String],
    requested_results: &[String],
) -> StudyPlan {
    let mut missing_parameters = base_model_missing(facts);
    if facts.spectrum_count == 0 || facts.referenced_spectrum_count == 0 {
        missing_parameters.push(missing(
            "model.spectrum",
            "模型必须包含被注入设备引用的有效 spectrum",
        ));
    }
    if !h_max.is_some_and(|value| value >= 2) {
        missing_parameters.push(missing("hMax", "必须明确至少为 2 的最大谐波次数"));
    }
    if monitor_elements.len() > MAX_ITEMS
        || monitor_elements
            .iter()
            .any(|name| !valid_identifier(name) || !facts.elements.contains(name))
    {
        missing_parameters.push(missing("monitorElements", "监视对象必须存在且最多 64 个"));
    }
    if !missing_parameters.is_empty() {
        let status = if missing_parameters
            .iter()
            .any(|value| value.field.starts_with("model."))
        {
            StudyStatus::MissingModelData
        } else {
            StudyStatus::NeedsClarification
        };
        return plan(
            request,
            facts,
            status,
            Some("ds_harmonic_study"),
            Some(22),
            vec![],
            missing_parameters,
            vec![],
        );
    }
    finish_ready(
        request,
        facts,
        functions,
        "ds_harmonic_study",
        None,
        requested_results,
        &[],
        22,
        vec![],
    )
}

fn transient(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    functions: &StudyFunctionCatalog,
    subtype: Option<TransientSubtype>,
    dynamics: &Option<MotorDynamicsRequest>,
    requested_results: &[String],
) -> StudyPlan {
    if subtype.is_none() {
        return plan(
            request,
            facts,
            StudyStatus::NeedsClarification,
            None,
            None,
            vec![],
            vec![MissingParameter {
                field: "subtype".into(),
                reason: "暂态类型必须明确；当前仅支持电机—网络动态".into(),
                choices: vec!["motor_dynamics".into()],
            }],
            vec![],
        );
    }
    let mut missing_parameters = base_model_missing(facts);
    if facts.motor_count == 0 {
        missing_parameters.push(missing("model.motor", "电机动态要求模型包含 motor"));
    }
    if facts.storage_count != 0 {
        return plan(
            request,
            facts,
            StudyStatus::Unsupported,
            Some("ds_motor_dynamics"),
            Some(28),
            vec![],
            vec![],
            vec!["当前 ds_motor_dynamics 不接受非空独立 storage 表".into()],
        );
    }
    match dynamics {
        None => {
            missing_parameters.push(missing("motorDynamics", "需要电机动态时长、步长和事件声明"))
        }
        Some(value) => {
            if !value
                .duration_seconds
                .is_some_and(|number| number.is_finite() && number > 0.0)
            {
                missing_parameters.push(missing(
                    "motorDynamics.durationSeconds",
                    "仿真时长必须有限且大于零",
                ));
            }
            if !value
                .step_size_seconds
                .is_some_and(|number| number.is_finite() && number > 0.0)
            {
                missing_parameters.push(missing(
                    "motorDynamics.stepSizeSeconds",
                    "时间步长必须有限且大于零",
                ));
            } else if value
                .duration_seconds
                .zip(value.step_size_seconds)
                .is_some_and(|(duration, step)| step > duration)
            {
                missing_parameters.push(missing(
                    "motorDynamics.stepSizeSeconds",
                    "时间步长不能大于仿真时长",
                ));
            }
            match &value.events {
                None => missing_parameters.push(missing(
                    "motorDynamics.events",
                    "必须明确事件列表或明确无外部事件",
                )),
                Some(events) if events.len() > MAX_ITEMS => {
                    missing_parameters.push(missing("motorDynamics.events", "动态事件最多 64 个"))
                }
                Some(events) if !events.iter().all(|event| valid_event(event, facts)) => {
                    missing_parameters.push(missing(
                        "motorDynamics.events",
                        "事件的时间、母线或元件引用无效",
                    ));
                }
                Some(_) => {}
            }
        }
    }
    if !missing_parameters.is_empty() {
        let status = if missing_parameters
            .iter()
            .any(|value| value.field.starts_with("model."))
        {
            StudyStatus::MissingModelData
        } else {
            StudyStatus::NeedsClarification
        };
        return plan(
            request,
            facts,
            status,
            Some("ds_motor_dynamics"),
            Some(28),
            vec![],
            missing_parameters,
            vec![],
        );
    }
    finish_ready(
        request,
        facts,
        functions,
        "ds_motor_dynamics",
        None,
        requested_results,
        &[],
        28,
        vec![],
    )
}

fn inferred_solver(facts: &ModelStudyFacts) -> ResolvedSolver {
    if facts.voltage_source_count != 1
        || !facts.is_connected
        || facts.has_loops
        || facts.has_parallel_branches
        || facts.has_pv_generator
        || facts.has_gfm
    {
        ResolvedSolver::Newton
    } else {
        ResolvedSolver::Radial
    }
}

fn base_model_missing(facts: &ModelStudyFacts) -> Vec<MissingParameter> {
    let mut values = Vec::new();
    if facts.bus_count == 0 {
        values.push(missing("model.bus", "active 模型没有母线"));
    }
    if facts.voltage_source_count == 0 {
        values.push(missing("model.voltageSource", "active 模型没有固定电压源"));
    }
    values
}

fn valid_profile(profile: &QstsProfile) -> bool {
    match profile {
        QstsProfile::Constant {
            load_multiplier,
            generation_multiplier,
        } => load_multiplier.is_finite() && generation_multiplier.is_finite(),
        QstsProfile::Inline { points } => {
            !points.is_empty()
                && points.len() <= MAX_PROFILE_POINTS
                && points
                    .windows(2)
                    .all(|pair| pair[0].time_seconds < pair[1].time_seconds)
                && points.iter().all(|point| {
                    point.time_seconds.is_finite()
                        && point.time_seconds >= 0.0
                        && point.load_multiplier.is_finite()
                        && point.generation_multiplier.is_finite()
                })
        }
    }
}

fn valid_event(event: &MotorEvent, facts: &ModelStudyFacts) -> bool {
    match event {
        MotorEvent::FaultOn {
            time_seconds,
            event_id,
            bus,
            phases,
            zf_re,
            zf_im,
            ..
        } => {
            time_seconds.is_finite()
                && *time_seconds >= 0.0
                && valid_identifier(event_id)
                && facts.buses.get(bus).is_some_and(|terminals| {
                    !phases.is_empty() && phases.iter().all(|phase| terminals.contains(phase))
                })
                && zf_re.is_none_or(f64::is_finite)
                && zf_im.is_none_or(f64::is_finite)
        }
        MotorEvent::FaultOff {
            time_seconds,
            event_id,
        } => time_seconds.is_finite() && *time_seconds >= 0.0 && valid_identifier(event_id),
        MotorEvent::Topology {
            time_seconds,
            element,
            action,
        } => {
            time_seconds.is_finite()
                && *time_seconds >= 0.0
                && facts.elements.contains(element)
                && matches!(action.as_str(), "open" | "trip" | "close" | "reclose")
        }
        MotorEvent::Protection {
            time_seconds,
            device,
            element,
            action,
        } => {
            time_seconds.is_finite()
                && *time_seconds >= 0.0
                && valid_identifier(device)
                && facts.elements.contains(element)
                && matches!(action.as_str(), "open" | "trip" | "close" | "reclose")
        }
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_IDENTIFIER_BYTES && !value.chars().any(char::is_control)
}

#[allow(clippy::too_many_arguments)]
fn finish_ready(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    functions: &StudyFunctionCatalog,
    function_name: &str,
    solver: Option<ResolvedSolver>,
    requested_results: &[String],
    product_defaults: &[&str],
    expected_arity: usize,
    warnings: Vec<String>,
) -> StudyPlan {
    let Some(contract) = functions.get(function_name) else {
        return plan(
            request,
            facts,
            StudyStatus::Unsupported,
            Some(function_name),
            None,
            vec![],
            vec![],
            vec![format!("当前引擎 catalog 不包含 {function_name}")],
        );
    };
    if contract.arity != expected_arity {
        return plan(
            request,
            facts,
            StudyStatus::Unsupported,
            Some(function_name),
            Some(contract.arity),
            vec![],
            vec![],
            vec![format!(
                "当前引擎 {function_name} arity={}，期望 {expected_arity}",
                contract.arity
            )],
        );
    }
    let results = if requested_results.is_empty() {
        if product_defaults.is_empty() {
            vec![contract.default_result.clone()]
        } else {
            product_defaults
                .iter()
                .map(|value| (*value).into())
                .collect()
        }
    } else {
        requested_results.to_vec()
    };
    let unique = results.iter().collect::<BTreeSet<_>>();
    if results.len() > MAX_ITEMS
        || unique.len() != results.len()
        || results
            .iter()
            .any(|result| !contract.results.contains(result))
    {
        return plan(
            request,
            facts,
            StudyStatus::NeedsClarification,
            Some(function_name),
            Some(contract.arity),
            vec![],
            vec![MissingParameter {
                field: "requestedResults".into(),
                reason: "结果名必须唯一且属于当前函数 catalog".into(),
                choices: contract.results.iter().cloned().collect(),
            }],
            warnings,
        );
    }
    let ready = ReadyStudyPlan {
        model_version_id: facts.active_version_id.expect("active checked"),
        model_content_hash: facts.content_hash.clone().unwrap_or_default(),
        engine_snapshot_id: facts.engine_snapshot_id.clone(),
        input_aliases: facts.available_aliases.iter().cloned().collect(),
        model_input_order: model_input_order(facts),
        request: request.clone(),
        function: function_name.into(),
        arity: contract.arity,
        results: results.clone(),
        solver,
    };
    StudyPlan {
        status: StudyStatus::Ready,
        study_type: request.kind(),
        model_facts: facts.into(),
        input_aliases: facts.available_aliases.iter().cloned().collect(),
        model_input_order: model_input_order(facts),
        function: Some(function_name.into()),
        arity: Some(contract.arity),
        solver,
        results,
        missing_parameters: vec![],
        warnings,
        ready: Some(ready),
    }
}

#[allow(clippy::too_many_arguments)]
fn plan(
    request: &StudyRequest,
    facts: &ModelStudyFacts,
    status: StudyStatus,
    function: Option<&str>,
    arity: Option<usize>,
    results: Vec<String>,
    missing_parameters: Vec<MissingParameter>,
    warnings: Vec<String>,
) -> StudyPlan {
    StudyPlan {
        status,
        study_type: request.kind(),
        model_facts: facts.into(),
        input_aliases: facts.available_aliases.iter().cloned().collect(),
        model_input_order: model_input_order(facts),
        function: function.map(str::to_string),
        arity,
        solver: None,
        results,
        missing_parameters,
        warnings,
        ready: None,
    }
}

fn model_input_order(facts: &ModelStudyFacts) -> Vec<String> {
    STATIC_MODEL_TABLES
        .into_iter()
        .filter(|alias| facts.available_aliases.contains(*alias))
        .map(str::to_string)
        .collect()
}

fn missing(field: &str, reason: &str) -> MissingParameter {
    MissingParameter {
        field: field.into(),
        reason: reason.into(),
        choices: vec![],
    }
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};

    use serde_json::json;

    use super::super::*;

    fn functions() -> StudyFunctionCatalog {
        StudyFunctionCatalog::from_value(&json!({
            "schema_version": 2,
            "frame_functions": [
                {
                    "names": ["ds_radial_pf", "ds_newton_raphson_pf"],
                    "complete_arity": 21,
                    "default_result": "bus_voltage",
                    "results": ["bus_voltage", "bus_phase", "summary"]
                },
                {
                    "names": ["ds_radial_qsts", "ds_newton_raphson_qsts"],
                    "arity": 33,
                    "default_result": "bus_voltage",
                    "results": ["bus_voltage", "bus_phase", "summary"]
                },
                {
                    "names": ["ds_fault_study"],
                    "complete_arity": 22,
                    "default_result": "fault",
                    "results": ["fault", "branch_flow"]
                },
                {
                    "names": ["ds_harmonic_study"],
                    "complete_arity": 22,
                    "default_result": "bus_voltage",
                    "results": ["bus_voltage", "monitor"]
                },
                {
                    "names": ["ds_motor_dynamics"],
                    "arity": 28,
                    "default_result": "motor_state",
                    "results": ["bus_voltage", "motor_state", "event"]
                }
            ]
        }))
        .unwrap()
    }

    fn radial_facts() -> ModelStudyFacts {
        ModelStudyFacts {
            available_aliases: crate::adapters::STATIC_MODEL_TABLES
                .into_iter()
                .map(str::to_string)
                .collect(),
            active_version_id: Some(42),
            content_hash: Some("model-hash".into()),
            engine_snapshot_id: "snapshot-1".into(),
            bus_count: 3,
            voltage_source_count: 1,
            generator_count: 0,
            ibr_count: 0,
            motor_count: 1,
            storage_count: 0,
            spectrum_count: 1,
            referenced_spectrum_count: 1,
            fault_source_equivalent_count: 1,
            is_connected: true,
            has_loops: false,
            has_parallel_branches: false,
            has_pv_generator: false,
            has_gfm: false,
            buses: BTreeMap::from([
                (
                    "source".into(),
                    BTreeSet::from(["1".into(), "2".into(), "3".into()]),
                ),
                (
                    "bus5".into(),
                    BTreeSet::from(["1".into(), "2".into(), "3".into()]),
                ),
            ]),
            elements: BTreeSet::from(["line.l1".into(), "motor.m1".into()]),
        }
    }

    fn fields(plan: &StudyPlan) -> Vec<&str> {
        plan.missing_parameters
            .iter()
            .map(|value| value.field.as_str())
            .collect()
    }

    #[test]
    fn static_power_flow_is_ready_and_selects_radial_or_newton() {
        let request = StudyRequest::StaticPowerFlow {
            solver_preference: SolverPreference::Auto,
            requested_results: vec![],
        };
        let radial = evaluate(&request, &radial_facts(), &functions());
        assert_eq!(radial.status, StudyStatus::Ready);
        assert_eq!(radial.function.as_deref(), Some("ds_radial_pf"));
        assert_eq!(radial.results, vec!["bus_phase", "summary"]);
        assert!(radial.ready.is_some());
        assert_eq!(
            radial.input_aliases,
            crate::adapters::STATIC_MODEL_TABLES
                .into_iter()
                .map(str::to_string)
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        assert_eq!(
            radial.ready.as_ref().unwrap().input_aliases,
            radial.input_aliases
        );
        assert_eq!(
            radial.model_input_order,
            crate::adapters::STATIC_MODEL_TABLES
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            radial.ready.as_ref().unwrap().model_input_order,
            radial.model_input_order
        );

        let mut meshed = radial_facts();
        meshed.has_loops = true;
        let newton = evaluate(&request, &meshed, &functions());
        assert_eq!(newton.status, StudyStatus::Ready);
        assert_eq!(newton.function.as_deref(), Some("ds_newton_raphson_pf"));
        assert_eq!(newton.solver, Some(ResolvedSolver::Newton));
    }

    #[test]
    fn explicit_newton_is_respected_for_a_radial_network() {
        let request = StudyRequest::StaticPowerFlow {
            solver_preference: SolverPreference::Newton,
            requested_results: vec![],
        };

        let plan = evaluate(&request, &radial_facts(), &functions());

        assert_eq!(plan.status, StudyStatus::Ready);
        assert_eq!(plan.function.as_deref(), Some("ds_newton_raphson_pf"));
        assert_eq!(plan.solver, Some(ResolvedSolver::Newton));
        assert_eq!(plan.ready.as_ref().unwrap().request, request);
    }

    #[test]
    fn governed_study_reports_every_missing_canonical_alias() {
        let mut facts = radial_facts();
        facts.available_aliases = BTreeSet::from(["bus".to_string(), "line".to_string()]);

        let plan = evaluate(
            &StudyRequest::StaticPowerFlow {
                solver_preference: SolverPreference::Auto,
                requested_results: vec![],
            },
            &facts,
            &functions(),
        );

        assert_eq!(plan.status, StudyStatus::MissingModelData);
        assert!(plan.ready.is_none());
        assert_eq!(plan.input_aliases, vec!["bus", "line"]);
        assert_eq!(plan.model_input_order, vec!["bus", "line"]);
        assert_eq!(
            fields(&plan),
            crate::adapters::STATIC_MODEL_TABLES
                .into_iter()
                .filter(|name| !matches!(*name, "bus" | "line"))
                .map(|name| format!("model.tables.{name}"))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn qsts_reports_all_missing_time_and_profile_fields() {
        let plan = evaluate(
            &StudyRequest::Qsts {
                solver_preference: SolverPreference::Auto,
                mode: None,
                start_seconds: None,
                step_size_seconds: None,
                number: None,
                profile: None,
                requested_results: vec![],
            },
            &radial_facts(),
            &functions(),
        );
        assert_eq!(plan.status, StudyStatus::NeedsClarification);
        assert_eq!(
            fields(&plan),
            vec![
                "mode",
                "startSeconds",
                "stepSizeSeconds",
                "number",
                "profile"
            ]
        );
        assert!(plan.ready.is_none());

        let ready = evaluate(
            &StudyRequest::Qsts {
                solver_preference: SolverPreference::Auto,
                mode: Some(QstsMode::Daily),
                start_seconds: Some(0.0),
                step_size_seconds: Some(3600.0),
                number: Some(24),
                profile: Some(QstsProfile::Constant {
                    load_multiplier: 1.0,
                    generation_multiplier: 1.0,
                }),
                requested_results: vec![],
            },
            &radial_facts(),
            &functions(),
        );
        assert_eq!(ready.status, StudyStatus::Ready);
        assert_eq!(ready.function.as_deref(), Some("ds_radial_qsts"));

        let newton = evaluate(
            &StudyRequest::Qsts {
                solver_preference: SolverPreference::Newton,
                mode: Some(QstsMode::Daily),
                start_seconds: Some(0.0),
                step_size_seconds: Some(3600.0),
                number: Some(24),
                profile: Some(QstsProfile::Constant {
                    load_multiplier: 1.0,
                    generation_multiplier: 1.0,
                }),
                requested_results: vec![],
            },
            &radial_facts(),
            &functions(),
        );
        assert_eq!(newton.status, StudyStatus::Ready);
        assert_eq!(newton.function.as_deref(), Some("ds_newton_raphson_qsts"));
        assert_eq!(newton.solver, Some(ResolvedSolver::Newton));
    }

    #[test]
    fn fault_requires_explicit_location_kind_terminals_impedance_and_ideal_source_consent() {
        let mut facts = radial_facts();
        facts.fault_source_equivalent_count = 0;
        let plan = evaluate(
            &StudyRequest::Fault {
                cases: Some(vec![FaultCase {
                    bus: None,
                    terminal_map: None,
                    kind: None,
                    bolted: None,
                    zf: None,
                    xf: None,
                }]),
                allow_ideal_sources: None,
                requested_results: vec![],
            },
            &facts,
            &functions(),
        );
        assert_eq!(plan.status, StudyStatus::NeedsClarification);
        assert_eq!(
            fields(&plan),
            vec![
                "cases[0].bus",
                "cases[0].terminalMap",
                "cases[0].kind",
                "cases[0].impedance",
                "allowIdealSources"
            ]
        );
        assert!(plan.ready.is_none());
    }

    #[test]
    fn harmonic_distinguishes_missing_spectrum_from_missing_h_max() {
        let request = StudyRequest::Harmonic {
            h_max: None,
            monitor_elements: vec![],
            requested_results: vec![],
        };
        let missing_parameter = evaluate(&request, &radial_facts(), &functions());
        assert_eq!(missing_parameter.status, StudyStatus::NeedsClarification);
        assert_eq!(fields(&missing_parameter), vec!["hMax"]);

        let mut no_spectrum = radial_facts();
        no_spectrum.spectrum_count = 0;
        no_spectrum.referenced_spectrum_count = 0;
        let missing_model = evaluate(&request, &no_spectrum, &functions());
        assert_eq!(missing_model.status, StudyStatus::MissingModelData);
        assert_eq!(fields(&missing_model), vec!["model.spectrum", "hMax"]);
    }

    #[test]
    fn transient_requires_subtype_and_only_supports_motor_dynamics() {
        let unspecified = evaluate(
            &StudyRequest::Transient {
                subtype: None,
                motor_dynamics: None,
                requested_results: vec![],
            },
            &radial_facts(),
            &functions(),
        );
        assert_eq!(unspecified.status, StudyStatus::NeedsClarification);
        assert_eq!(fields(&unspecified), vec!["subtype"]);

        let unsupported = evaluate(
            &StudyRequest::Transient {
                subtype: Some(TransientSubtype::Emt),
                motor_dynamics: None,
                requested_results: vec![],
            },
            &radial_facts(),
            &functions(),
        );
        assert_eq!(unsupported.status, StudyStatus::Unsupported);
        assert!(unsupported.function.is_none());

        let missing_events = evaluate(
            &StudyRequest::Transient {
                subtype: Some(TransientSubtype::MotorDynamics),
                motor_dynamics: Some(MotorDynamicsRequest {
                    duration_seconds: Some(2.0),
                    step_size_seconds: Some(0.01),
                    events: None,
                }),
                requested_results: vec![],
            },
            &radial_facts(),
            &functions(),
        );
        assert_eq!(missing_events.status, StudyStatus::NeedsClarification);
        assert_eq!(fields(&missing_events), vec!["motorDynamics.events"]);

        let explicit_no_events = evaluate(
            &StudyRequest::Transient {
                subtype: Some(TransientSubtype::MotorDynamics),
                motor_dynamics: Some(MotorDynamicsRequest {
                    duration_seconds: Some(2.0),
                    step_size_seconds: Some(0.01),
                    events: Some(vec![]),
                }),
                requested_results: vec![],
            },
            &radial_facts(),
            &functions(),
        );
        assert_eq!(explicit_no_events.status, StudyStatus::Ready);
        assert_eq!(
            explicit_no_events.function.as_deref(),
            Some("ds_motor_dynamics")
        );
    }

    #[test]
    fn catalog_rejects_duplicate_results() {
        let result = StudyFunctionCatalog::from_value(&json!({
            "frame_functions": [{
                "names": ["ds_radial_pf"],
                "complete_arity": 21,
                "default_result": "bus_phase",
                "results": ["bus_phase", "bus_phase"]
            }]
        }));
        assert_eq!(
            result.unwrap_err(),
            StudyPlanningError::InvalidCatalog(
                "函数 ds_radial_pf 的 results 包含重复名称: bus_phase".into()
            )
        );
    }

    #[test]
    fn catalog_loads_governed_studies_from_the_real_dsa_function_section() {
        let catalog = StudyFunctionCatalog::from_value(&json!({
            "schema_version": 2,
            "frame_functions": [
                {"name": "select", "arity": 2}
            ],
            "dsa_functions": [
                {"names": ["ds_dss_plan"], "allowed_arities": [1, 2]},
                {
                    "names": ["ds_radial_pf", "ds_newton_raphson_pf"],
                    "complete_arity": 21,
                    "default_result": "bus_voltage",
                    "results": ["bus_voltage", "bus_phase", "summary"]
                }
            ]
        }))
        .unwrap();

        assert_eq!(catalog.get("ds_radial_pf").unwrap().arity, 21);
        assert_eq!(
            catalog.get("ds_newton_raphson_pf").unwrap().default_result,
            "bus_voltage"
        );
        assert!(catalog.get("select").is_none());
        assert!(catalog.get("ds_dss_plan").is_none());
    }

    #[test]
    fn request_boundaries_reject_excessive_or_inconsistent_values() {
        let duplicate_results = evaluate(
            &StudyRequest::StaticPowerFlow {
                solver_preference: SolverPreference::Auto,
                requested_results: vec!["summary".into(), "summary".into()],
            },
            &radial_facts(),
            &functions(),
        );
        assert_eq!(duplicate_results.status, StudyStatus::NeedsClarification);
        assert_eq!(fields(&duplicate_results), vec!["requestedResults"]);

        let mut meshed = radial_facts();
        meshed.has_parallel_branches = true;
        let incompatible_radial = evaluate(
            &StudyRequest::StaticPowerFlow {
                solver_preference: SolverPreference::Radial,
                requested_results: vec![],
            },
            &meshed,
            &functions(),
        );
        assert_eq!(incompatible_radial.status, StudyStatus::NeedsClarification);
        assert_eq!(fields(&incompatible_radial), vec!["solverPreference"]);

        let excessive_terminals = evaluate(
            &StudyRequest::Fault {
                cases: Some(vec![FaultCase {
                    bus: Some("bus5".into()),
                    terminal_map: Some(vec!["1".into(); 65]),
                    kind: Some(FaultKind::Slg),
                    bolted: Some(true),
                    zf: None,
                    xf: None,
                }]),
                allow_ideal_sources: None,
                requested_results: vec![],
            },
            &radial_facts(),
            &functions(),
        );
        assert_eq!(excessive_terminals.status, StudyStatus::NeedsClarification);
        assert_eq!(fields(&excessive_terminals), vec!["cases[0].terminalMap"]);

        let invalid_timeline = evaluate(
            &StudyRequest::Transient {
                subtype: Some(TransientSubtype::MotorDynamics),
                motor_dynamics: Some(MotorDynamicsRequest {
                    duration_seconds: Some(1.0),
                    step_size_seconds: Some(2.0),
                    events: Some(vec![]),
                }),
                requested_results: vec![],
            },
            &radial_facts(),
            &functions(),
        );
        assert_eq!(invalid_timeline.status, StudyStatus::NeedsClarification);
        assert_eq!(
            fields(&invalid_timeline),
            vec!["motorDynamics.stepSizeSeconds"]
        );
    }

    #[test]
    fn missing_or_drifted_catalog_function_is_unsupported() {
        let request = StudyRequest::StaticPowerFlow {
            solver_preference: SolverPreference::Auto,
            requested_results: vec![],
        };
        let missing = evaluate(&request, &radial_facts(), &StudyFunctionCatalog::default());
        assert_eq!(missing.status, StudyStatus::Unsupported);

        let wrong_arity = StudyFunctionCatalog::from_value(&json!({
            "frame_functions": [{
                "names": ["ds_radial_pf"],
                "arity": 20,
                "default_result": "bus_phase",
                "results": ["bus_phase", "summary"]
            }]
        }))
        .unwrap();
        let drifted = evaluate(&request, &radial_facts(), &wrong_arity);
        assert_eq!(drifted.status, StudyStatus::Unsupported);
        assert_eq!(drifted.arity, Some(20));
    }
}
