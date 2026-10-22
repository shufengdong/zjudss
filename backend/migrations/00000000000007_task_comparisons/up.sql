CREATE TABLE task_comparisons (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 baseline_task_id INTEGER NOT NULL REFERENCES simulation_tasks(id) ON DELETE RESTRICT,
 candidate_task_id INTEGER NOT NULL REFERENCES simulation_tasks(id) ON DELETE RESTRICT,
 compatibility_json TEXT NOT NULL,
 analysis_run_id INTEGER NOT NULL UNIQUE REFERENCES result_analysis_runs(id) ON DELETE RESTRICT,
 created_at_ms BIGINT NOT NULL,
 CHECK (baseline_task_id <> candidate_task_id)
);
CREATE INDEX idx_comparisons_session_time ON task_comparisons(session_id, created_at_ms, id);
