CREATE TABLE ingestions (
    id                   INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id         INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    client_batch_id      TEXT NOT NULL,
    origin_kind          TEXT NOT NULL CHECK (origin_kind IN ('upload', 'example')),
    source_ref_json      TEXT,
    base_version_id      INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
    published_version_id INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
    status               TEXT NOT NULL CHECK (status IN ('accepted', 'extracting', 'inferring', 'validating', 'succeeded', 'completed_with_errors', 'failed')),
    stages_json          TEXT NOT NULL,
    error_code           TEXT,
    error_message        TEXT,
    created_at_ms        INTEGER NOT NULL,
    updated_at_ms        INTEGER NOT NULL,
    UNIQUE(workspace_id, client_batch_id)
);

CREATE TABLE ingestion_artifacts (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    ingestion_id       INTEGER NOT NULL REFERENCES ingestions(id) ON DELETE CASCADE,
    client_file_id     TEXT NOT NULL,
    artifact_id        INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
    file_name          TEXT NOT NULL,
    relative_path      TEXT,
    media_type         TEXT,
    role               TEXT NOT NULL CHECK (role IN ('uploaded', 'example', 'extracted', 'normalized')),
    source_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE SET NULL,
    sheet_name         TEXT,
    status             TEXT NOT NULL CHECK (status IN ('accepted', 'processing', 'ready', 'failed')),
    error_code         TEXT,
    error_message      TEXT,
    created_at_ms      INTEGER NOT NULL,
    updated_at_ms      INTEGER NOT NULL,
    UNIQUE(ingestion_id, client_file_id, role, sheet_name)
);

CREATE INDEX idx_ingestions_workspace_time ON ingestions(workspace_id, created_at_ms DESC, id DESC);
CREATE INDEX idx_ingestion_artifacts_ingestion ON ingestion_artifacts(ingestion_id, id);
