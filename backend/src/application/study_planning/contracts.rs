use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SolverPreference {
    #[default]
    Auto,
    Radial,
    Newton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResolvedSolver {
    Radial,
    Newton,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyKind {
    StaticPowerFlow,
    Qsts,
    Fault,
    Harmonic,
    Transient,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QstsMode {
    Daily,
    Yearly,
    Duty,
    PeakDay,
    LoadDuration,
    Ld1,
    Ld2,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QstsProfilePoint {
    pub time_seconds: f64,
    pub load_multiplier: f64,
    pub generation_multiplier: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum QstsProfile {
    Constant {
        load_multiplier: f64,
        generation_multiplier: f64,
    },
    Inline {
        points: Vec<QstsProfilePoint>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FaultKind {
    #[serde(rename = "slg")]
    Slg,
    #[serde(rename = "ll")]
    Ll,
    #[serde(rename = "llg")]
    Llg,
    #[serde(rename = "2pg")]
    TwoPg,
    #[serde(rename = "3p")]
    ThreeP,
    #[serde(rename = "3pg")]
    ThreePg,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FaultCase {
    pub bus: Option<String>,
    pub terminal_map: Option<Vec<String>>,
    pub kind: Option<FaultKind>,
    pub bolted: Option<bool>,
    pub zf: Option<f64>,
    pub xf: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransientSubtype {
    MotorDynamics,
    Emt,
    TransientStability,
    InverterControl,
    Other,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum MotorEvent {
    FaultOn {
        time_seconds: f64,
        event_id: String,
        bus: String,
        phases: Vec<String>,
        fault_kind: FaultKind,
        zf_re: Option<f64>,
        zf_im: Option<f64>,
    },
    FaultOff {
        time_seconds: f64,
        event_id: String,
    },
    Topology {
        time_seconds: f64,
        element: String,
        action: String,
    },
    Protection {
        time_seconds: f64,
        device: String,
        element: String,
        action: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MotorDynamicsRequest {
    pub duration_seconds: Option<f64>,
    pub step_size_seconds: Option<f64>,
    pub events: Option<Vec<MotorEvent>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "studyType",
    rename_all = "snake_case",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum StudyRequest {
    StaticPowerFlow {
        #[serde(default)]
        solver_preference: SolverPreference,
        #[serde(default)]
        requested_results: Vec<String>,
    },
    Qsts {
        #[serde(default)]
        solver_preference: SolverPreference,
        mode: Option<QstsMode>,
        start_seconds: Option<f64>,
        step_size_seconds: Option<f64>,
        number: Option<u32>,
        profile: Option<QstsProfile>,
        #[serde(default)]
        requested_results: Vec<String>,
    },
    Fault {
        cases: Option<Vec<FaultCase>>,
        allow_ideal_sources: Option<bool>,
        #[serde(default)]
        requested_results: Vec<String>,
    },
    Harmonic {
        h_max: Option<u32>,
        #[serde(default)]
        monitor_elements: Vec<String>,
        #[serde(default)]
        requested_results: Vec<String>,
    },
    Transient {
        subtype: Option<TransientSubtype>,
        motor_dynamics: Option<MotorDynamicsRequest>,
        #[serde(default)]
        requested_results: Vec<String>,
    },
}

impl StudyRequest {
    pub fn kind(&self) -> StudyKind {
        match self {
            Self::StaticPowerFlow { .. } => StudyKind::StaticPowerFlow,
            Self::Qsts { .. } => StudyKind::Qsts,
            Self::Fault { .. } => StudyKind::Fault,
            Self::Harmonic { .. } => StudyKind::Harmonic,
            Self::Transient { .. } => StudyKind::Transient,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ModelStudyFacts {
    pub available_aliases: BTreeSet<String>,
    pub active_version_id: Option<i32>,
    pub content_hash: Option<String>,
    pub engine_snapshot_id: String,
    pub bus_count: usize,
    pub voltage_source_count: usize,
    pub generator_count: usize,
    pub ibr_count: usize,
    pub motor_count: usize,
    pub storage_count: usize,
    pub spectrum_count: usize,
    pub referenced_spectrum_count: usize,
    pub fault_source_equivalent_count: usize,
    pub is_connected: bool,
    pub has_loops: bool,
    pub has_parallel_branches: bool,
    pub has_pv_generator: bool,
    pub has_gfm: bool,
    pub buses: BTreeMap<String, BTreeSet<String>>,
    pub elements: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStudySummary {
    pub active_version_id: Option<String>,
    pub content_hash: Option<String>,
    pub bus_count: usize,
    pub voltage_source_count: usize,
    pub generator_count: usize,
    pub ibr_count: usize,
    pub motor_count: usize,
    pub storage_count: usize,
    pub spectrum_count: usize,
    pub referenced_spectrum_count: usize,
    pub fault_source_equivalent_count: usize,
    pub is_connected: bool,
    pub has_loops: bool,
    pub has_parallel_branches: bool,
    pub has_pv_generator: bool,
    pub has_gfm: bool,
}

impl From<&ModelStudyFacts> for ModelStudySummary {
    fn from(value: &ModelStudyFacts) -> Self {
        Self {
            active_version_id: value
                .active_version_id
                .map(|id| format!("model-version:{id}")),
            content_hash: value.content_hash.clone(),
            bus_count: value.bus_count,
            voltage_source_count: value.voltage_source_count,
            generator_count: value.generator_count,
            ibr_count: value.ibr_count,
            motor_count: value.motor_count,
            storage_count: value.storage_count,
            spectrum_count: value.spectrum_count,
            referenced_spectrum_count: value.referenced_spectrum_count,
            fault_source_equivalent_count: value.fault_source_equivalent_count,
            is_connected: value.is_connected,
            has_loops: value.has_loops,
            has_parallel_branches: value.has_parallel_branches,
            has_pv_generator: value.has_pv_generator,
            has_gfm: value.has_gfm,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum StudyStatus {
    Ready,
    NeedsClarification,
    MissingModelData,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MissingParameter {
    pub field: String,
    pub reason: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StudyFunctionContract {
    pub name: String,
    pub arity: usize,
    pub default_result: String,
    pub results: BTreeSet<String>,
}

#[derive(Debug, Clone, Default)]
pub struct StudyFunctionCatalog {
    functions: BTreeMap<String, StudyFunctionContract>,
}

impl StudyFunctionCatalog {
    pub fn from_value(value: &Value) -> Result<Self, StudyPlanningError> {
        const GOVERNED: &[&str] = &[
            "ds_radial_pf",
            "ds_newton_raphson_pf",
            "ds_radial_qsts",
            "ds_newton_raphson_qsts",
            "ds_fault_study",
            "ds_harmonic_study",
            "ds_motor_dynamics",
        ];
        let mut entries = Vec::new();
        for section in ["frame_functions", "dsa_functions"] {
            if let Some(values) = value.get(section).and_then(Value::as_array) {
                entries.extend(values);
            }
        }
        if entries.is_empty() {
            return Err(StudyPlanningError::InvalidCatalog(
                "缺少 frame_functions/dsa_functions".into(),
            ));
        }
        let mut functions = BTreeMap::new();
        for entry in entries {
            let names = match (entry.get("names"), entry.get("name")) {
                (Some(Value::Array(names)), _) => names
                    .iter()
                    .map(|name| {
                        name.as_str().ok_or_else(|| {
                            StudyPlanningError::InvalidCatalog("函数名不是字符串".into())
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                (Some(_), _) => {
                    return Err(StudyPlanningError::InvalidCatalog(
                        "函数 names 不是数组".into(),
                    ));
                }
                (None, Some(Value::String(name))) => vec![name.as_str()],
                (None, Some(_)) => {
                    return Err(StudyPlanningError::InvalidCatalog(
                        "函数名不是字符串".into(),
                    ));
                }
                (None, None) => {
                    return Err(StudyPlanningError::InvalidCatalog(
                        "函数缺少 name/names".into(),
                    ));
                }
            };
            let names = names
                .into_iter()
                .filter(|name| GOVERNED.contains(name))
                .collect::<Vec<_>>();
            if names.is_empty() {
                continue;
            }
            let arity = entry
                .get("complete_arity")
                .or_else(|| entry.get("arity"))
                .and_then(Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| StudyPlanningError::InvalidCatalog("函数缺少有效 arity".into()))?;
            let default_result = entry
                .get("default_result")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            let result_values = entry
                .get("results")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let mut results = BTreeSet::new();
            for result in result_values {
                let result = result
                    .as_str()
                    .ok_or_else(|| StudyPlanningError::InvalidCatalog("结果名不是字符串".into()))?;
                if !results.insert(result.to_string()) {
                    return Err(StudyPlanningError::InvalidCatalog(format!(
                        "函数 {} 的 results 包含重复名称: {result}",
                        names.first().copied().unwrap_or("<unknown>")
                    )));
                }
            }
            for name in names {
                let contract = StudyFunctionContract {
                    name: name.to_string(),
                    arity,
                    default_result: default_result.clone(),
                    results: results.clone(),
                };
                if functions.insert(name.to_string(), contract).is_some() {
                    return Err(StudyPlanningError::InvalidCatalog(format!(
                        "函数重复: {name}"
                    )));
                }
            }
        }
        Ok(Self { functions })
    }

    pub fn get(&self, name: &str) -> Option<&StudyFunctionContract> {
        self.functions.get(name)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadyStudyPlan {
    pub model_version_id: i32,
    pub model_content_hash: String,
    pub engine_snapshot_id: String,
    pub input_aliases: Vec<String>,
    pub model_input_order: Vec<String>,
    pub request: StudyRequest,
    pub function: String,
    pub arity: usize,
    pub results: Vec<String>,
    pub solver: Option<ResolvedSolver>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StudyPlan {
    pub status: StudyStatus,
    pub study_type: StudyKind,
    pub model_facts: ModelStudySummary,
    pub input_aliases: Vec<String>,
    pub model_input_order: Vec<String>,
    pub function: Option<String>,
    pub arity: Option<usize>,
    pub solver: Option<ResolvedSolver>,
    pub results: Vec<String>,
    pub missing_parameters: Vec<MissingParameter>,
    pub warnings: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ready: Option<ReadyStudyPlan>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StudyPlanningError {
    NotFound,
    InvalidCatalog(String),
    ModelReadFailed(String),
}

impl StudyPlanningError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::InvalidCatalog(_) => "study_catalog_invalid",
            Self::ModelReadFailed(_) => "study_model_read_failed",
        }
    }
}

impl std::fmt::Display for StudyPlanningError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound => write!(formatter, "资源不存在"),
            Self::InvalidCatalog(message) => write!(formatter, "研究函数目录无效: {message}"),
            Self::ModelReadFailed(message) => write!(formatter, "研究模型读取失败: {message}"),
        }
    }
}

impl std::error::Error for StudyPlanningError {}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{
        FaultCase, FaultKind, MotorDynamicsRequest, MotorEvent, QstsMode, QstsProfile,
        SolverPreference, StudyRequest, TransientSubtype,
    };

    #[test]
    fn study_request_json_preserves_camel_case_static_power_flow_fields() {
        let request: StudyRequest = serde_json::from_value(json!({
            "studyType": "static_power_flow",
            "solverPreference": "newton",
            "requestedResults": ["bus_phase", "summary"]
        }))
        .unwrap();

        assert_eq!(
            request,
            StudyRequest::StaticPowerFlow {
                solver_preference: SolverPreference::Newton,
                requested_results: vec!["bus_phase".into(), "summary".into()],
            }
        );
        assert_eq!(
            serde_json::to_value(request).unwrap(),
            json!({
                "studyType": "static_power_flow",
                "solverPreference": "newton",
                "requestedResults": ["bus_phase", "summary"]
            })
        );
    }

    #[test]
    fn study_request_json_preserves_camel_case_qsts_fields() {
        let request: StudyRequest = serde_json::from_value(json!({
            "studyType": "qsts",
            "solverPreference": "newton",
            "mode": "daily",
            "startSeconds": 0.0,
            "stepSizeSeconds": 900.0,
            "number": 96,
            "profile": {
                "kind": "constant",
                "loadMultiplier": 1.0,
                "generationMultiplier": 0.8
            },
            "requestedResults": ["summary"]
        }))
        .unwrap();

        assert_eq!(
            request,
            StudyRequest::Qsts {
                solver_preference: SolverPreference::Newton,
                mode: Some(QstsMode::Daily),
                start_seconds: Some(0.0),
                step_size_seconds: Some(900.0),
                number: Some(96),
                profile: Some(QstsProfile::Constant {
                    load_multiplier: 1.0,
                    generation_multiplier: 0.8,
                }),
                requested_results: vec!["summary".into()],
            }
        );
    }

    #[test]
    fn study_request_json_rejects_unknown_or_misspelled_fields() {
        let error = serde_json::from_value::<StudyRequest>(json!({
            "studyType": "static_power_flow",
            "solver_preference": "newton"
        }))
        .unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn study_request_json_preserves_camel_case_fields_for_other_studies() {
        let fault: StudyRequest = serde_json::from_value(json!({
            "studyType": "fault",
            "cases": [{
                "bus": "bus1",
                "terminalMap": ["1"],
                "kind": "slg",
                "bolted": true,
                "zf": null,
                "xf": null
            }],
            "allowIdealSources": true,
            "requestedResults": ["fault_result"]
        }))
        .unwrap();
        assert_eq!(
            fault,
            StudyRequest::Fault {
                cases: Some(vec![FaultCase {
                    bus: Some("bus1".into()),
                    terminal_map: Some(vec!["1".into()]),
                    kind: Some(FaultKind::Slg),
                    bolted: Some(true),
                    zf: None,
                    xf: None,
                }]),
                allow_ideal_sources: Some(true),
                requested_results: vec!["fault_result".into()],
            }
        );

        let harmonic: StudyRequest = serde_json::from_value(json!({
            "studyType": "harmonic",
            "hMax": 25,
            "monitorElements": ["line.l1"],
            "requestedResults": ["bus_voltage"]
        }))
        .unwrap();
        assert_eq!(
            harmonic,
            StudyRequest::Harmonic {
                h_max: Some(25),
                monitor_elements: vec!["line.l1".into()],
                requested_results: vec!["bus_voltage".into()],
            }
        );

        let transient: StudyRequest = serde_json::from_value(json!({
            "studyType": "transient",
            "subtype": "motor_dynamics",
            "motorDynamics": {
                "durationSeconds": 1.0,
                "stepSizeSeconds": 0.01,
                "events": [{
                    "kind": "fault_on",
                    "timeSeconds": 0.1,
                    "eventId": "f1",
                    "bus": "bus1",
                    "phases": ["1"],
                    "faultKind": "slg",
                    "zfRe": 0.0,
                    "zfIm": 0.0
                }]
            },
            "requestedResults": ["event"]
        }))
        .unwrap();
        assert_eq!(
            transient,
            StudyRequest::Transient {
                subtype: Some(TransientSubtype::MotorDynamics),
                motor_dynamics: Some(MotorDynamicsRequest {
                    duration_seconds: Some(1.0),
                    step_size_seconds: Some(0.01),
                    events: Some(vec![MotorEvent::FaultOn {
                        time_seconds: 0.1,
                        event_id: "f1".into(),
                        bus: "bus1".into(),
                        phases: vec!["1".into()],
                        fault_kind: FaultKind::Slg,
                        zf_re: Some(0.0),
                        zf_im: Some(0.0),
                    }]),
                }),
                requested_results: vec!["event".into()],
            }
        );
    }
}
