CREATE TABLE workspaces (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id        INTEGER NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
    active_version_id INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
    draft_version_id  INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
    created_at_ms     INTEGER NOT NULL,
    updated_at_ms     INTEGER NOT NULL,
    CHECK (active_version_id IS NULL OR draft_version_id IS NULL OR active_version_id != draft_version_id)
);

CREATE TABLE model_versions (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    workspace_id      INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
    parent_version_id INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
    status            TEXT NOT NULL CHECK (status IN ('collecting', 'ready', 'invalid')),
    source_kind       TEXT NOT NULL CHECK (source_kind IN ('rss', 'rssbundle', 'dataframes', 'mixed')),
    origin_kind       TEXT NOT NULL CHECK (origin_kind IN ('upload', 'example', 'generated')),
    manifest_json     TEXT NOT NULL,
    patch_json        TEXT,
    content_hash      TEXT NOT NULL,
    created_at_ms     INTEGER NOT NULL,
    UNIQUE(workspace_id, content_hash)
);
CREATE INDEX idx_versions_workspace_time ON model_versions(workspace_id, created_at_ms DESC, id DESC);

CREATE TABLE version_artifacts (
    id                 INTEGER PRIMARY KEY AUTOINCREMENT,
    version_id         INTEGER NOT NULL REFERENCES model_versions(id) ON DELETE CASCADE,
    artifact_id        INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
    role               TEXT NOT NULL CHECK (role IN ('entry', 'source', 'data', 'input', 'generated', 'manifest')),
    relative_path      TEXT,
    display_name       TEXT NOT NULL,
    source_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE SET NULL,
    sheet_name         TEXT
);
CREATE UNIQUE INDEX idx_version_artifact_path
    ON version_artifacts(version_id, relative_path)
    WHERE relative_path IS NOT NULL;

CREATE TABLE dataframe_bindings (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    version_artifact_id INTEGER NOT NULL UNIQUE REFERENCES version_artifacts(id) ON DELETE CASCADE,
    table_kind          TEXT,
    confidence          REAL CHECK (confidence IS NULL OR (confidence >= 0 AND confidence <= 1)),
    resolution          TEXT NOT NULL CHECK (resolution IN ('pending', 'inferred', 'user_confirmed', 'rejected')),
    columns_json        TEXT NOT NULL,
    evidence_json       TEXT NOT NULL
);

CREATE TRIGGER validate_workspace_active_insert
BEFORE INSERT ON workspaces
WHEN NEW.active_version_id IS NOT NULL
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM model_versions
        WHERE id = NEW.active_version_id AND workspace_id = NEW.id AND status = 'ready'
    ) THEN RAISE(ABORT, 'invalid active model version') END;
END;

CREATE TRIGGER validate_workspace_active_update
BEFORE UPDATE OF active_version_id ON workspaces
WHEN NEW.active_version_id IS NOT NULL
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM model_versions
        WHERE id = NEW.active_version_id AND workspace_id = NEW.id AND status = 'ready'
    ) THEN RAISE(ABORT, 'invalid active model version') END;
END;

CREATE TRIGGER validate_workspace_draft_update
BEFORE UPDATE OF draft_version_id ON workspaces
WHEN NEW.draft_version_id IS NOT NULL
BEGIN
    SELECT CASE WHEN NOT EXISTS (
        SELECT 1 FROM model_versions
        WHERE id = NEW.draft_version_id AND workspace_id = NEW.id
    ) THEN RAISE(ABORT, 'invalid draft model version') END;
END;

CREATE TRIGGER immutable_model_versions
BEFORE UPDATE OF workspace_id, status, source_kind, origin_kind, manifest_json, patch_json, content_hash, created_at_ms
ON model_versions
BEGIN
    SELECT RAISE(ABORT, 'model versions are immutable');
END;

CREATE TRIGGER immutable_version_artifacts
BEFORE UPDATE ON version_artifacts
BEGIN
    SELECT RAISE(ABORT, 'version artifacts are immutable');
END;

CREATE TRIGGER immutable_dataframe_bindings
BEFORE UPDATE ON dataframe_bindings
BEGIN
    SELECT RAISE(ABORT, 'dataframe bindings are immutable');
END;
