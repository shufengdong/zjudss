DROP TRIGGER IF EXISTS sessions_require_scope_update;
DROP TRIGGER IF EXISTS sessions_require_scope_insert;
DROP INDEX IF EXISTS idx_sessions_storage_scope;
DROP INDEX IF EXISTS idx_sessions_access_scope;
ALTER TABLE sessions DROP COLUMN storage_scope_id;
ALTER TABLE sessions DROP COLUMN access_scope_key;
ALTER TABLE sessions DROP COLUMN access_scope_kind;
DROP TABLE storage_scopes;
