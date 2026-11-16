DROP TABLE IF EXISTS dataset_lineage_snapshots;
DROP TABLE IF EXISTS result_analysis_inputs;
CREATE TABLE result_analysis_inputs (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 analysis_run_id INTEGER NOT NULL REFERENCES result_analysis_runs(id) ON DELETE CASCADE,
 dataset_id INTEGER REFERENCES result_datasets(id) ON DELETE RESTRICT,
 resource_id TEXT REFERENCES data_resources(id) ON DELETE RESTRICT,
 alias TEXT NOT NULL,
 ordinal INTEGER NOT NULL,
 source_name TEXT NOT NULL,
 source_schema_json TEXT NOT NULL,
 source_schema_hash TEXT NOT NULL,
 source_content_hash TEXT NOT NULL,
 source_producer_json TEXT NOT NULL,
 CHECK((dataset_id IS NOT NULL AND resource_id IS NULL) OR (dataset_id IS NULL AND resource_id IS NOT NULL))
);
CREATE TABLE dataset_lineage_snapshots (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 derived_dataset_id INTEGER NOT NULL REFERENCES result_datasets(id) ON DELETE CASCADE,
 source_dataset_id INTEGER REFERENCES result_datasets(id) ON DELETE RESTRICT,
 source_resource_id TEXT REFERENCES data_resources(id) ON DELETE RESTRICT,
 source_name TEXT NOT NULL,
 source_schema_json TEXT NOT NULL,
 source_schema_hash TEXT NOT NULL,
 source_content_hash TEXT NOT NULL,
 source_producer_json TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 CHECK((source_dataset_id IS NOT NULL AND source_resource_id IS NULL) OR (source_dataset_id IS NULL AND source_resource_id IS NOT NULL))
);