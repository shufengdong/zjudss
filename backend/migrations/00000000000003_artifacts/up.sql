ALTER TABLE storage_scopes ADD COLUMN artifact_limit INTEGER NOT NULL DEFAULT 20000 CHECK (artifact_limit > 0);
ALTER TABLE storage_scopes ADD COLUMN byte_limit INTEGER NOT NULL DEFAULT 21474836480 CHECK (byte_limit > 0);
ALTER TABLE storage_scopes ADD COLUMN warning_ratio REAL NOT NULL DEFAULT 0.8 CHECK (warning_ratio > 0 AND warning_ratio <= 1);
ALTER TABLE storage_scopes ADD COLUMN used_artifacts INTEGER NOT NULL DEFAULT 0 CHECK (used_artifacts >= 0);
ALTER TABLE storage_scopes ADD COLUMN used_bytes INTEGER NOT NULL DEFAULT 0 CHECK (used_bytes >= 0);
ALTER TABLE storage_scopes ADD COLUMN updated_at_ms INTEGER NOT NULL DEFAULT 0;

CREATE TABLE artifacts (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    sha256            TEXT    NOT NULL UNIQUE CHECK (length(sha256) = 64),
    size_bytes        INTEGER NOT NULL CHECK (size_bytes >= 0),
    storage_path      TEXT    NOT NULL UNIQUE,
    format            TEXT    NOT NULL,
    media_type        TEXT,
    created_at_ms     INTEGER NOT NULL
);

CREATE TABLE storage_scope_artifacts (
    storage_scope_id INTEGER NOT NULL REFERENCES storage_scopes(id) ON DELETE CASCADE,
    artifact_id      INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
    ref_count        INTEGER NOT NULL CHECK (ref_count > 0),
    created_at_ms    INTEGER NOT NULL,
    PRIMARY KEY (storage_scope_id, artifact_id)
);
CREATE INDEX idx_scope_artifacts_artifact ON storage_scope_artifacts(artifact_id);

CREATE TABLE artifact_gc_failures (
    relative_path TEXT PRIMARY KEY,
    error         TEXT    NOT NULL,
    attempts      INTEGER NOT NULL DEFAULT 1,
    updated_at_ms INTEGER NOT NULL
);
