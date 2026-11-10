mod app;
mod control;
mod engine_fixture;
mod faults;
mod run_control;
mod scenario_engine;
mod scripted_agent;
mod scripted_author;

pub use app::{
    E2eAppDependencies, E2eAppOptions, E2eAppProfile, E2eCatalogMode, assemble_e2e_app,
    build_e2e_app,
};
pub use control::{ControlState, E2eConfig, E2eConfigError, control_router, run_e2e_server};
pub use engine_fixture::{engine_fixture_root, prepare_engine_fixture};
pub use faults::{FaultInjector, FaultPoint};
pub use run_control::RunControlGate;
pub(crate) use run_control::{
    finish_result_publication, pause_before_registration, pause_result_publication,
};
pub use scenario_engine::{
    DatasetFixture, EngineScenario, IdDatasetFixture, RecordedEngineCall, ScenarioEngine,
};
pub use scripted_agent::{
    AgentTrace, AgentTraceCall, ScriptScenario, ScriptTurn, ScriptedAgentFactory,
};
pub use scripted_author::{AuthorTurn, ScriptedAuthor};
