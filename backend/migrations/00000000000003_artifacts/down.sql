DROP TABLE IF EXISTS artifact_gc_failures;
DROP INDEX IF EXISTS idx_scope_artifacts_artifact;
DROP TABLE IF EXISTS storage_scope_artifacts;
DROP TABLE IF EXISTS artifacts;

ALTER TABLE storage_scopes DROP COLUMN updated_at_ms;
ALTER TABLE storage_scopes DROP COLUMN used_bytes;
ALTER TABLE storage_scopes DROP COLUMN used_artifacts;
ALTER TABLE storage_scopes DROP COLUMN warning_ratio;
ALTER TABLE storage_scopes DROP COLUMN byte_limit;
ALTER TABLE storage_scopes DROP COLUMN artifact_limit;
