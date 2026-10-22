ALTER TABLE simulation_tasks ADD COLUMN model_version_id INTEGER REFERENCES model_versions(id);
ALTER TABLE simulation_tasks ADD COLUMN study_spec_json TEXT;
ALTER TABLE simulation_tasks ADD COLUMN script_artifact_id INTEGER REFERENCES artifacts(id);
ALTER TABLE simulation_tasks ADD COLUMN agent_bundle_version TEXT;
ALTER TABLE simulation_tasks ADD COLUMN agent_skill_refs_json TEXT;
ALTER TABLE simulation_tasks ADD COLUMN engine_contract_version TEXT;

CREATE TABLE result_analysis_runs (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
    request_message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
    intent_json TEXT NOT NULL,
    script_artifact_id INTEGER NOT NULL REFERENCES artifacts(id),
    status TEXT NOT NULL CHECK (status IN ('pending','running','succeeded','failed')),
    stage TEXT NOT NULL,
    error_json TEXT,
    agent_bundle_version TEXT NOT NULL,
    agent_skill_refs_json TEXT NOT NULL,
    engine_contract_version TEXT NOT NULL,
    created_at_ms BIGINT NOT NULL,
    started_at_ms BIGINT,
    finished_at_ms BIGINT
);

CREATE TABLE result_analysis_inputs (
    analysis_run_id INTEGER NOT NULL REFERENCES result_analysis_runs(id) ON DELETE CASCADE,
    dataset_id INTEGER NOT NULL REFERENCES result_datasets(id) ON DELETE RESTRICT,
    alias TEXT NOT NULL,
    ordinal INTEGER NOT NULL,
    PRIMARY KEY (analysis_run_id, dataset_id),
    UNIQUE (analysis_run_id, alias),
    UNIQUE (analysis_run_id, ordinal)
);

CREATE TABLE result_datasets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id INTEGER REFERENCES simulation_tasks(id) ON DELETE CASCADE,
    analysis_run_id INTEGER REFERENCES result_analysis_runs(id) ON DELETE CASCADE,
    artifact_id INTEGER NOT NULL REFERENCES artifacts(id),
    result_name TEXT NOT NULL,
    schema_json TEXT NOT NULL,
    schema_hash TEXT NOT NULL,
    row_count BIGINT NOT NULL CHECK (row_count >= 0),
    size_bytes BIGINT NOT NULL CHECK (size_bytes >= 0),
    created_at_ms BIGINT NOT NULL,
    CHECK ((task_id IS NOT NULL) <> (analysis_run_id IS NOT NULL)),
    UNIQUE (task_id, result_name),
    UNIQUE (analysis_run_id, result_name)
);

CREATE TABLE dataset_views (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    dataset_id INTEGER NOT NULL REFERENCES result_datasets(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    kind TEXT NOT NULL CHECK (kind IN ('line','bar','scatter','heatmap','boxplot','histogram')),
    spec_json TEXT NOT NULL,
    is_default BOOLEAN NOT NULL DEFAULT 0,
    created_at_ms BIGINT NOT NULL,
    UNIQUE (dataset_id, name)
);

CREATE INDEX idx_simulation_tasks_model_version ON simulation_tasks(model_version_id);
CREATE INDEX idx_result_datasets_task ON result_datasets(task_id, id);
CREATE INDEX idx_result_datasets_analysis ON result_datasets(analysis_run_id, id);
CREATE INDEX idx_dataset_views_dataset ON dataset_views(dataset_id, id);
CREATE INDEX idx_result_analysis_runs_session ON result_analysis_runs(session_id, id);
