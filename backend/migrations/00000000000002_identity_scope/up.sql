CREATE TABLE storage_scopes (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    kind          TEXT    NOT NULL CHECK (kind IN ('instance', 'owner', 'tenant')),
    scope_key     TEXT    NOT NULL,
    created_at_ms INTEGER NOT NULL,
    UNIQUE (kind, scope_key)
);

INSERT INTO storage_scopes(kind, scope_key, created_at_ms)
VALUES ('instance', 'instance', 0);

ALTER TABLE sessions ADD COLUMN access_scope_kind TEXT NOT NULL DEFAULT '';
ALTER TABLE sessions ADD COLUMN access_scope_key TEXT NOT NULL DEFAULT '';
ALTER TABLE sessions ADD COLUMN storage_scope_id INTEGER REFERENCES storage_scopes(id);

UPDATE sessions
SET access_scope_kind = 'instance',
    access_scope_key = 'instance',
    storage_scope_id = (
        SELECT id FROM storage_scopes WHERE kind = 'instance' AND scope_key = 'instance'
    );

CREATE INDEX idx_sessions_access_scope
ON sessions(access_scope_kind, access_scope_key, updated_at_ms);
CREATE INDEX idx_sessions_storage_scope ON sessions(storage_scope_id);

CREATE TRIGGER sessions_require_scope_insert
BEFORE INSERT ON sessions
WHEN NEW.access_scope_kind NOT IN ('instance', 'owner', 'tenant')
  OR length(NEW.access_scope_key) = 0
  OR NEW.storage_scope_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'session scope is required');
END;

CREATE TRIGGER sessions_require_scope_update
BEFORE UPDATE OF access_scope_kind, access_scope_key, storage_scope_id ON sessions
WHEN NEW.access_scope_kind NOT IN ('instance', 'owner', 'tenant')
  OR length(NEW.access_scope_key) = 0
  OR NEW.storage_scope_id IS NULL
BEGIN
    SELECT RAISE(ABORT, 'session scope is required');
END;
