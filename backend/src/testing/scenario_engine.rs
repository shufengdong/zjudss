use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use arrow_array::builder::{Float64Builder, ListBuilder};
use arrow_array::{ArrayRef, Float64Array, RecordBatch, StringArray};
use arrow_schema::{DataType, Field, Schema};
use async_trait::async_trait;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Notify;

use crate::adapters::StubEngine;
use crate::adapters::{
    AdapterError, EngineRunOutput, ErrorCategory, ImportSpec, ImportedModel, InspectOutcome,
    InspectSpec, OutputLimits, RunMode, RunSpec, SimEngine, validate_outputs,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatasetFixture {
    pub name: String,
    pub rows: Vec<(String, f64)>,
    pub value_unit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IdDatasetFixture {
    pub name: String,
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum EngineScenario {
    Success(Vec<DatasetFixture>),
    IdSuccess(Vec<IdDatasetFixture>),
    BusVoltageSuccess,
    InspectFailure,
    RunFailure,
    Failure {
        code: String,
        stderr: Vec<u8>,
        exit_code: i32,
    },
    ContractError {
        details: serde_json::Value,
    },
    MonitorError,
    Cancelled {
        barrier_id: String,
    },
    Timeout {
        barrier_id: String,
    },
    Empty,
    CorruptParquet,
    Oversize,
    EscapingPath,
    Symlink,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecordedEngineCall {
    pub sequence: usize,
    pub operation: String,
    pub mode: Option<String>,
    pub scenario: String,
    pub input_tree: PathBuf,
    pub result_dir: PathBuf,
}

#[derive(Default)]
struct BarrierState {
    entered: AtomicBool,
    released: AtomicBool,
    entered_notify: Notify,
    released_notify: Notify,
}

impl BarrierState {
    async fn block(&self) {
        self.entered.store(true, Ordering::Release);
        self.entered_notify.notify_waiters();
        while !self.released.load(Ordering::Acquire) {
            self.released_notify.notified().await;
        }
    }
    async fn wait_until_entered(&self) {
        while !self.entered.load(Ordering::Acquire) {
            self.entered_notify.notified().await;
        }
    }
    fn release(&self) {
        self.released.store(true, Ordering::Release);
        self.released_notify.notify_waiters();
    }
}

struct Inner {
    scenarios: Mutex<VecDeque<EngineScenario>>,
    calls: Mutex<Vec<RecordedEngineCall>>,
    barriers: Mutex<HashMap<String, Arc<BarrierState>>>,
    limits: OutputLimits,
}

#[derive(Clone)]
pub struct ScenarioEngine {
    inner: Arc<Inner>,
}

impl Default for ScenarioEngine {
    fn default() -> Self {
        Self::with_output_limits(OutputLimits::default())
    }
}

impl ScenarioEngine {
    pub fn with_output_limits(limits: OutputLimits) -> Self {
        Self {
            inner: Arc::new(Inner {
                scenarios: Mutex::new(VecDeque::new()),
                calls: Mutex::new(Vec::new()),
                barriers: Mutex::new(HashMap::new()),
                limits,
            }),
        }
    }

    pub fn enqueue(&self, scenario: EngineScenario) {
        if let EngineScenario::Timeout { barrier_id } | EngineScenario::Cancelled { barrier_id } =
            &scenario
        {
            self.inner
                .barriers
                .lock()
                .expect("engine barriers")
                .entry(barrier_id.clone())
                .or_default();
        }
        self.inner
            .scenarios
            .lock()
            .expect("engine scenarios")
            .push_back(scenario);
    }

    pub fn calls(&self) -> Vec<RecordedEngineCall> {
        self.inner.calls.lock().expect("engine calls").clone()
    }

    pub fn release(&self, barrier_id: &str) -> bool {
        let barrier = self
            .inner
            .barriers
            .lock()
            .expect("engine barriers")
            .get(barrier_id)
            .cloned();
        if let Some(barrier) = barrier {
            barrier.release();
            true
        } else {
            false
        }
    }

    pub async fn wait_until_blocked(&self, barrier_id: &str) -> bool {
        let barrier = self
            .inner
            .barriers
            .lock()
            .expect("engine barriers")
            .get(barrier_id)
            .cloned();
        if let Some(barrier) = barrier {
            barrier.wait_until_entered().await;
            true
        } else {
            false
        }
    }

    async fn run_scenario(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        spec.validate()?;
        let scenario = self
            .inner
            .scenarios
            .lock()
            .expect("engine scenarios")
            .pop_front()
            .ok_or_else(|| failure("scenario_missing", "没有可用的 engine 场景"))?;
        let scenario_name = scenario_name(&scenario);
        {
            let mut calls = self.inner.calls.lock().expect("engine calls");
            let sequence = calls.len() + 1;
            calls.push(RecordedEngineCall {
                sequence,
                operation: "run".into(),
                mode: Some(format!("{:?}", spec.mode).to_ascii_lowercase()),
                scenario: scenario_name.into(),
                input_tree: spec.input_tree.clone(),
                result_dir: spec.result_dir.clone(),
            });
        }
        std::fs::create_dir_all(&spec.result_dir)
            .map_err(|error| io_failure("result_dir_create_failed", error))?;
        match scenario {
            EngineScenario::Success(fixtures) => {
                for fixture in fixtures {
                    write_fixture(&spec.result_dir, &fixture)?;
                }
                validate_outputs(&spec.result_dir, self.inner.limits)
            }
            EngineScenario::IdSuccess(fixtures) => {
                for fixture in fixtures {
                    write_id_fixture(&spec.result_dir, &fixture)?;
                }
                validate_outputs(&spec.result_dir, self.inner.limits)
            }
            EngineScenario::BusVoltageSuccess => {
                write_bus_voltage_fixture(&spec.result_dir)?;
                validate_outputs(&spec.result_dir, self.inner.limits)
            }
            EngineScenario::InspectFailure => Err(failure(
                "scenario_mismatch",
                "InspectFailure 只能由 inspect 消费",
            )),
            EngineScenario::RunFailure => Err(failure(
                "scenario_run_failed",
                "RSS parse error: unexpected token\nat /srv/private/runs/secret/input/script.rss:12:4",
            )
            .with_diagnostics(serde_json::json!({"exitCode": 7}))),
            EngineScenario::Failure {
                code,
                stderr,
                exit_code,
            } => Err(failure(code, "仿真按场景失败").with_diagnostics(
                serde_json::json!({
                    "exitCode": exit_code,
                    "stderr": captured_stream(stderr),
                }),
            )),
            EngineScenario::ContractError { details } => Err(failure(
                "result_contract_invalid",
                "结果合同按场景失败",
            )
            .with_diagnostics(details)),
            EngineScenario::MonitorError => Err(failure(
                "result_monitor_failed",
                "结果监控按场景失败",
            )),
            EngineScenario::Cancelled { barrier_id } => {
                let barrier = self
                    .inner
                    .barriers
                    .lock()
                    .expect("engine barriers")
                    .get(&barrier_id)
                    .cloned()
                    .ok_or_else(|| failure("scenario_barrier_missing", barrier_id))?;
                barrier.block().await;
                Err(failure("engine_cancelled", "仿真按场景取消"))
            }
            EngineScenario::Timeout { barrier_id } => {
                let barrier = self
                    .inner
                    .barriers
                    .lock()
                    .expect("engine barriers")
                    .get(&barrier_id)
                    .cloned()
                    .ok_or_else(|| failure("scenario_barrier_missing", barrier_id))?;
                barrier.block().await;
                Err(AdapterError::new(
                    "engine_timeout",
                    ErrorCategory::Timeout,
                    "仿真按场景超时",
                ))
            }
            EngineScenario::Empty => validate_outputs(&spec.result_dir, self.inner.limits),
            EngineScenario::CorruptParquet => {
                std::fs::write(spec.result_dir.join("corrupt.parquet"), b"not parquet")
                    .map_err(|error| io_failure("scenario_write_failed", error))?;
                validate_outputs(&spec.result_dir, self.inner.limits)
            }
            EngineScenario::Oversize => {
                let file = std::fs::File::create(spec.result_dir.join("oversize.parquet"))
                    .map_err(|error| io_failure("scenario_write_failed", error))?;
                file.set_len(self.inner.limits.max_file_bytes.saturating_add(1))
                    .map_err(|error| io_failure("scenario_write_failed", error))?;
                validate_outputs(&spec.result_dir, self.inner.limits)
            }
            EngineScenario::EscapingPath => {
                std::fs::create_dir(spec.result_dir.join("escape.parquet"))
                    .map_err(|error| io_failure("scenario_write_failed", error))?;
                validate_outputs(&spec.result_dir, self.inner.limits)
            }
            EngineScenario::Symlink => {
                let outside = spec.result_dir.with_extension("outside.parquet");
                std::fs::write(&outside, b"outside")
                    .map_err(|error| io_failure("scenario_write_failed", error))?;
                #[cfg(unix)]
                std::os::unix::fs::symlink(&outside, spec.result_dir.join("linked.parquet"))
                    .map_err(|error| io_failure("scenario_write_failed", error))?;
                #[cfg(windows)]
                std::os::windows::fs::symlink_file(
                    &outside,
                    spec.result_dir.join("linked.parquet"),
                )
                .map_err(|error| io_failure("scenario_write_failed", error))?;
                validate_outputs(&spec.result_dir, self.inner.limits)
            }
        }
    }
}

#[async_trait]
impl SimEngine for ScenarioEngine {
    async fn inspect(&self, spec: InspectSpec) -> Result<InspectOutcome, AdapterError> {
        spec.validate()?;
        let mut calls = self.inner.calls.lock().expect("engine calls");
        let sequence = calls.len() + 1;
        calls.push(RecordedEngineCall {
            sequence,
            operation: "inspect".into(),
            mode: None,
            scenario: "inspect".into(),
            input_tree: spec.input_tree,
            result_dir: PathBuf::new(),
        });
        drop(calls);
        let is_failure = matches!(
            self.inner
                .scenarios
                .lock()
                .expect("engine scenarios")
                .front(),
            Some(EngineScenario::InspectFailure)
        );
        if is_failure {
            self.inner
                .scenarios
                .lock()
                .expect("engine scenarios")
                .pop_front();
            Err(failure("scenario_inspect_failed", "模型检查按场景失败"))
        } else {
            Ok(InspectOutcome {
                ok: true,
                summary: "scenario model ok".into(),
                diagnostics: None,
            })
        }
    }

    async fn run_dataframe(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        if !matches!(spec.mode, RunMode::Dataframe | RunMode::Rss) {
            return Err(failure("run_mode_invalid", "dataframe 入口模式错误"));
        }
        self.run_scenario(spec).await
    }

    async fn run_bundle(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        if spec.mode != RunMode::Bundle {
            return Err(failure("run_mode_invalid", "bundle 入口模式错误"));
        }
        self.run_scenario(spec).await
    }

    async fn run_analysis(&self, spec: RunSpec) -> Result<EngineRunOutput, AdapterError> {
        if spec.mode != RunMode::Analysis {
            return Err(failure("run_mode_invalid", "analysis 入口模式错误"));
        }
        self.run_scenario(spec).await
    }

    async fn import_model(&self, spec: ImportSpec) -> Result<ImportedModel, AdapterError> {
        StubEngine.import_model(spec).await
    }
}

fn write_fixture(
    result_dir: &std::path::Path,
    fixture: &DatasetFixture,
) -> Result<(), AdapterError> {
    if fixture.name.is_empty()
        || !fixture.name.chars().enumerate().all(|(index, value)| {
            value == '_'
                || value.is_ascii_alphanumeric() && (index > 0 || value.is_ascii_alphabetic())
        })
    {
        return Err(failure("scenario_fixture_invalid", "非法数据集名称"));
    }
    let mut value_metadata = HashMap::new();
    if let Some(unit) = &fixture.value_unit {
        value_metadata.insert("unit".to_string(), unit.clone());
    }
    let schema = Arc::new(Schema::new(vec![
        Field::new("key", DataType::Utf8, false),
        Field::new("value", DataType::Float64, false).with_metadata(value_metadata),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(
                fixture
                    .rows
                    .iter()
                    .map(|(key, _)| key.as_str())
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
            Arc::new(Float64Array::from(
                fixture
                    .rows
                    .iter()
                    .map(|(_, value)| *value)
                    .collect::<Vec<_>>(),
            )) as ArrayRef,
        ],
    )
    .map_err(|error| failure("scenario_fixture_invalid", error.to_string()))?;
    let file = std::fs::File::create(result_dir.join(format!("{}.parquet", fixture.name)))
        .map_err(|error| io_failure("scenario_write_failed", error))?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .set_created_by("scenario-engine-v1".to_string())
        .build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(properties))
        .map_err(|error| failure("scenario_write_failed", error.to_string()))?;
    writer
        .write(&batch)
        .and_then(|_| writer.close().map(|_| ()))
        .map_err(|error| failure("scenario_write_failed", error.to_string()))
}

fn write_id_fixture(
    result_dir: &std::path::Path,
    fixture: &IdDatasetFixture,
) -> Result<(), AdapterError> {
    validate_fixture_name(&fixture.name)?;
    let schema = Arc::new(Schema::new(vec![Field::new("id", DataType::Utf8, false)]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![Arc::new(StringArray::from(fixture.ids.clone())) as ArrayRef],
    )
    .map_err(|error| failure("scenario_fixture_invalid", error.to_string()))?;
    let file = std::fs::File::create(result_dir.join(format!("{}.parquet", fixture.name)))
        .map_err(|error| io_failure("scenario_write_failed", error))?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .set_created_by("scenario-engine-v1".to_string())
        .build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(properties))
        .map_err(|error| failure("scenario_write_failed", error.to_string()))?;
    writer
        .write(&batch)
        .and_then(|_| writer.close().map(|_| ()))
        .map_err(|error| failure("scenario_write_failed", error.to_string()))
}

fn write_bus_voltage_fixture(result_dir: &std::path::Path) -> Result<(), AdapterError> {
    let mut value_metadata = HashMap::new();
    value_metadata.insert("unit".to_string(), "V".to_string());
    let element = Arc::new(Field::new("element", DataType::Float64, false));
    let list_type = DataType::List(element.clone());
    let schema = Arc::new(Schema::new(vec![
        Field::new("bus", DataType::Utf8, true),
        Field::new("v_mag", list_type.clone(), false).with_metadata(value_metadata.clone()),
        Field::new("v_ang", list_type, false).with_metadata(value_metadata),
    ]));
    let mut magnitudes = ListBuilder::new(Float64Builder::new()).with_field(element.clone());
    magnitudes.values().append_value(1.0);
    magnitudes.append(true);
    let mut angles = ListBuilder::new(Float64Builder::new()).with_field(element);
    angles.values().append_value(0.0);
    angles.append(true);
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(StringArray::from(vec![Some("source")])) as ArrayRef,
            Arc::new(magnitudes.finish()) as ArrayRef,
            Arc::new(angles.finish()) as ArrayRef,
        ],
    )
    .map_err(|error| failure("scenario_fixture_invalid", error.to_string()))?;
    let file = std::fs::File::create(result_dir.join("bus_voltage.parquet"))
        .map_err(|error| io_failure("scenario_write_failed", error))?;
    let properties = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .set_created_by("scenario-engine-v1".to_string())
        .build();
    let mut writer = ArrowWriter::try_new(file, schema, Some(properties))
        .map_err(|error| failure("scenario_write_failed", error.to_string()))?;
    writer
        .write(&batch)
        .map_err(|error| failure("scenario_write_failed", error.to_string()))?;
    writer
        .close()
        .map_err(|error| failure("scenario_write_failed", error.to_string()))?;
    Ok(())
}

fn validate_fixture_name(name: &str) -> Result<(), AdapterError> {
    if name.is_empty()
        || !name.chars().enumerate().all(|(index, value)| {
            value == '_'
                || value.is_ascii_alphanumeric() && (index > 0 || value.is_ascii_alphabetic())
        })
    {
        return Err(failure("scenario_fixture_invalid", "非法数据集名称"));
    }
    Ok(())
}

fn failure(code: impl Into<String>, message: impl Into<String>) -> AdapterError {
    AdapterError::new(code, ErrorCategory::Internal, message)
}

fn io_failure(code: impl Into<String>, error: std::io::Error) -> AdapterError {
    AdapterError::new(code, ErrorCategory::Io, error.to_string())
}

fn scenario_name(scenario: &EngineScenario) -> &'static str {
    match scenario {
        EngineScenario::Success(_) => "success",
        EngineScenario::IdSuccess(_) => "id_success",
        EngineScenario::BusVoltageSuccess => "bus_voltage_success",
        EngineScenario::InspectFailure => "inspect_failure",
        EngineScenario::RunFailure => "run_failure",
        EngineScenario::Failure { .. } => "failure",
        EngineScenario::ContractError { .. } => "contract_error",
        EngineScenario::MonitorError => "monitor_error",
        EngineScenario::Cancelled { .. } => "cancelled",
        EngineScenario::Timeout { .. } => "timeout",
        EngineScenario::Empty => "empty",
        EngineScenario::CorruptParquet => "corrupt_parquet",
        EngineScenario::Oversize => "oversize",
        EngineScenario::EscapingPath => "escaping_path",
        EngineScenario::Symlink => "symlink",
    }
}

fn captured_stream(retained: Vec<u8>) -> crate::adapters::CapturedStream {
    let hash = hex::encode(Sha256::digest(&retained));
    crate::adapters::CapturedStream {
        total_bytes: retained.len() as u64,
        retained,
        full_sha256: hash.clone(),
        retained_sha256: hash,
        truncated: false,
    }
}
