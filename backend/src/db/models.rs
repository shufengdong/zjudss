//! 表对应的 Rust 结构体：`Queryable`（读整行）与 `Insertable`（写，不含自增主键）。

// Diesel 2.x：派生宏（Queryable/Insertable）只从 crate 根重导出（默认 feature
// `with-deprecated`），prelude 仅含 trait 与 DSL；故显式导入。
use super::schema::{
    artifacts, curve_data, messages, sessions, simulation_tasks, storage_scope_artifacts,
    storage_scopes, task_files, task_results,
};
#[allow(deprecated)]
use diesel::{Insertable, Queryable};

// ---- sessions ----

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = sessions)]
pub struct Session {
    pub id: i32,
    pub title: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub access_scope_kind: String,
    pub access_scope_key: String,
    pub storage_scope_id: Option<i32>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = sessions)]
pub struct NewSession {
    pub title: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    pub access_scope_kind: String,
    pub access_scope_key: String,
    pub storage_scope_id: Option<i32>,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = storage_scopes)]
pub struct StorageScope {
    pub id: i32,
    pub kind: String,
    pub scope_key: String,
    pub created_at_ms: i64,
    pub artifact_limit: i64,
    pub byte_limit: i64,
    pub warning_ratio: f64,
    pub used_artifacts: i64,
    pub used_bytes: i64,
    pub updated_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = storage_scopes)]
pub struct NewStorageScope {
    pub kind: String,
    pub scope_key: String,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = artifacts)]
pub struct Artifact {
    pub id: i32,
    pub sha256: String,
    pub size_bytes: i64,
    pub storage_path: String,
    pub format: String,
    pub media_type: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = artifacts)]
pub struct NewArtifact {
    pub sha256: String,
    pub size_bytes: i64,
    pub storage_path: String,
    pub format: String,
    pub media_type: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Queryable, Insertable, Debug, Clone)]
#[diesel(table_name = storage_scope_artifacts)]
pub struct StorageScopeArtifact {
    pub storage_scope_id: i32,
    pub artifact_id: i32,
    pub ref_count: i64,
    pub created_at_ms: i64,
}

#[derive(Queryable, Debug, Clone)]
pub struct ArtifactGcFailure {
    pub relative_path: String,
    pub error: String,
    pub attempts: i64,
    pub updated_at_ms: i64,
}

// ---- simulation_tasks ----

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = simulation_tasks)]
pub struct SimulationTask {
    pub id: i32,
    pub session_id: i32,
    pub status: String,
    pub stage: Option<String>,
    pub study_type: Option<String>,
    pub params_json: String,
    pub exit_code: Option<i32>,
    pub error_json: Option<String>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = simulation_tasks)]
pub struct NewSimulationTask {
    pub session_id: i32,
    pub status: String,
    pub stage: Option<String>,
    pub study_type: Option<String>,
    pub params_json: String,
    pub exit_code: Option<i32>,
    pub error_json: Option<String>,
    pub created_at_ms: i64,
    pub started_at_ms: Option<i64>,
    pub finished_at_ms: Option<i64>,
}

// ---- messages ----

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = messages)]
pub struct Message {
    pub id: i32,
    pub session_id: i32,
    pub task_id: Option<i32>,
    pub role: String,
    pub kind: String,
    pub content: String,
    pub payload_json: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = messages)]
pub struct NewMessage {
    pub session_id: i32,
    pub task_id: Option<i32>,
    pub role: String,
    pub kind: String,
    pub content: String,
    pub payload_json: Option<String>,
    pub created_at_ms: i64,
}

// ---- task_files ----

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = task_files)]
pub struct TaskFile {
    pub id: i32,
    pub task_id: i32,
    pub role: String,
    pub path: String,
    pub filename: String,
    pub format: Option<String>,
    pub result_name: Option<String>,
    pub size_bytes: Option<i64>,
    pub sha256: Option<String>,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = task_files)]
pub struct NewTaskFile {
    pub task_id: i32,
    pub role: String,
    pub path: String,
    pub filename: String,
    pub format: Option<String>,
    pub result_name: Option<String>,
    pub size_bytes: Option<i64>,
    pub sha256: Option<String>,
    pub created_at_ms: i64,
}

// ---- task_results ----

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = task_results)]
pub struct TaskResult {
    pub id: i32,
    pub task_id: i32,
    pub result_name: String,
    pub row_count: i32,
    pub converged: Option<bool>,
    pub violation_count: Option<i32>,
    pub p_loss_w: Option<f64>,
    pub q_loss_var: Option<f64>,
    pub max_violation_ratio: Option<f64>,
    pub metrics_json: String,
    pub created_at_ms: i64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = task_results)]
pub struct NewTaskResult {
    pub task_id: i32,
    pub result_name: String,
    pub row_count: i32,
    pub converged: Option<bool>,
    pub violation_count: Option<i32>,
    pub p_loss_w: Option<f64>,
    pub q_loss_var: Option<f64>,
    pub max_violation_ratio: Option<f64>,
    pub metrics_json: String,
    pub created_at_ms: i64,
}

// ---- curve_data ----

#[derive(Queryable, Debug, Clone)]
#[diesel(table_name = curve_data)]
pub struct CurveDatum {
    pub id: i32,
    pub task_id: i32,
    pub result_name: String,
    pub series_key: String,
    pub time_s: f64,
    pub variable: String,
    pub value: f64,
}

#[derive(Insertable, Debug, Clone)]
#[diesel(table_name = curve_data)]
pub struct NewCurveDatum {
    pub task_id: i32,
    pub result_name: String,
    pub series_key: String,
    pub time_s: f64,
    pub variable: String,
    pub value: f64,
}
