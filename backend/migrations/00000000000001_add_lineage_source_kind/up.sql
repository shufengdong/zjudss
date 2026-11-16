ALTER TABLE result_analysis_inputs ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'dataset';
ALTER TABLE result_analysis_inputs ADD COLUMN model_version_id INTEGER;
ALTER TABLE result_analysis_inputs ADD COLUMN model_dataframe_id INTEGER;

ALTER TABLE dataset_lineage_snapshots ADD COLUMN source_kind TEXT NOT NULL DEFAULT 'dataset';
ALTER TABLE dataset_lineage_snapshots ADD COLUMN model_version_id INTEGER;
ALTER TABLE dataset_lineage_snapshots ADD COLUMN model_dataframe_id INTEGER;