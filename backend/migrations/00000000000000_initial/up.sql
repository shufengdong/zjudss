-- zjudss V4 initial schema. Only empty-database initialization is supported.

CREATE TABLE storage_scopes (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 kind TEXT NOT NULL CHECK(kind='user'),
 scope_key TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 artifact_limit INTEGER NOT NULL DEFAULT 20000 CHECK(artifact_limit>0),
 byte_limit INTEGER NOT NULL DEFAULT 21474836480 CHECK(byte_limit>0),
 warning_ratio REAL NOT NULL DEFAULT 0.8 CHECK(warning_ratio>0 AND warning_ratio<=1),
 used_artifacts INTEGER NOT NULL DEFAULT 0 CHECK(used_artifacts>=0),
 used_bytes INTEGER NOT NULL DEFAULT 0 CHECK(used_bytes>=0),
 reserved_bytes INTEGER NOT NULL DEFAULT 0 CHECK(reserved_bytes>=0),
 updated_at_ms INTEGER NOT NULL DEFAULT 0,
 UNIQUE(kind,scope_key)
);

CREATE TABLE users (
 id TEXT PRIMARY KEY,
 name TEXT NOT NULL CHECK(length(name)>0),
 password_hash TEXT NOT NULL,
 storage_scope_id INTEGER NOT NULL UNIQUE REFERENCES storage_scopes(id) ON DELETE RESTRICT,
 created_at_ms INTEGER NOT NULL
);

CREATE TABLE auth_sessions (
 token_hash TEXT PRIMARY KEY CHECK(length(token_hash)=64),
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 created_at_ms INTEGER NOT NULL,
 last_seen_at_ms INTEGER NOT NULL,
 expires_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_auth_sessions_user ON auth_sessions(user_id);
CREATE INDEX idx_auth_sessions_expiry ON auth_sessions(expires_at_ms);

CREATE TABLE system_metadata (
 key TEXT PRIMARY KEY,
 value TEXT NOT NULL
);

CREATE TABLE sessions (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 title TEXT NOT NULL DEFAULT '',
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE RESTRICT,
 title_source TEXT NOT NULL DEFAULT 'default' CHECK(title_source IN ('default','auto','user')),
 deletion_status TEXT NOT NULL DEFAULT 'live' CHECK(deletion_status IN ('live','deleting','deleted')),
 deleted_at_ms INTEGER,
 CHECK((deletion_status='live' AND deleted_at_ms IS NULL)
    OR deletion_status='deleting'
    OR (deletion_status='deleted' AND deleted_at_ms IS NOT NULL))
);
CREATE INDEX idx_sessions_updated ON sessions(updated_at_ms);
CREATE INDEX idx_sessions_user_updated ON sessions(user_id,updated_at_ms);

CREATE TABLE artifacts (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 sha256 TEXT NOT NULL UNIQUE CHECK(length(sha256)=64),
 size_bytes INTEGER NOT NULL CHECK(size_bytes>=0),
 storage_path TEXT NOT NULL UNIQUE,
 format TEXT NOT NULL,
 media_type TEXT,
 created_at_ms INTEGER NOT NULL
);
CREATE TABLE storage_scope_artifacts (
 storage_scope_id INTEGER NOT NULL REFERENCES storage_scopes(id) ON DELETE CASCADE,
 artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
 ref_count INTEGER NOT NULL CHECK(ref_count>0),
 created_at_ms INTEGER NOT NULL,
 PRIMARY KEY(storage_scope_id,artifact_id)
);
CREATE INDEX idx_scope_artifacts_artifact ON storage_scope_artifacts(artifact_id);
CREATE TABLE artifact_gc_failures (
 relative_path TEXT PRIMARY KEY,
 error TEXT NOT NULL,
 attempts INTEGER NOT NULL DEFAULT 1,
 updated_at_ms INTEGER NOT NULL
);

CREATE TABLE engine_resource_snapshots (
 id TEXT PRIMARY KEY,
 engine_contract_version TEXT NOT NULL,
 catalog_hash TEXT NOT NULL CHECK(length(catalog_hash)=64),
 functions_schema_version INTEGER NOT NULL CHECK(functions_schema_version>0),
 study_facades_schema_version INTEGER NOT NULL CHECK(study_facades_schema_version>0),
 case_projections_schema_version INTEGER NOT NULL CHECK(case_projections_schema_version>0),
 dataframes_schema_version INTEGER NOT NULL CHECK(dataframes_schema_version>0),
 dtype_grammar_version INTEGER NOT NULL CHECK(dtype_grammar_version>0),
 is_current INTEGER NOT NULL DEFAULT 0 CHECK(is_current IN (0,1)),
 snapshot_json TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_one_current_engine_resource_snapshot
 ON engine_resource_snapshots(is_current) WHERE is_current=1;

CREATE TABLE workspaces (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
 active_version_id INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL
);
CREATE TABLE model_versions (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
 description TEXT NOT NULL CHECK(length(trim(description))>0 AND length(description)<=200),
 content_hash TEXT NOT NULL,
 source_summary_json TEXT NOT NULL,
 engine_contract_version TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 deleted_at_ms INTEGER,
 tombstone_json TEXT
);
CREATE INDEX idx_versions_workspace_time ON model_versions(workspace_id,created_at_ms DESC,id DESC);
CREATE UNIQUE INDEX idx_live_version_content
 ON model_versions(workspace_id,content_hash) WHERE deleted_at_ms IS NULL;

CREATE TABLE workspace_drafts (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 workspace_id INTEGER NOT NULL UNIQUE REFERENCES workspaces(id) ON DELETE CASCADE,
 base_version_id INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
 revision INTEGER NOT NULL CHECK(revision>0),
 diagnostics_json TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL
);
CREATE TABLE draft_dataframes (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 draft_id INTEGER NOT NULL REFERENCES workspace_drafts(id) ON DELETE CASCADE,
 table_name TEXT NOT NULL CHECK(length(table_name)>0 AND lower(table_name)<>'para'),
 artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
 schema_json TEXT NOT NULL,
 row_count INTEGER NOT NULL CHECK(row_count>=0),
 content_hash TEXT NOT NULL CHECK(length(content_hash)=64),
 created_at_ms INTEGER NOT NULL,
 UNIQUE(draft_id,table_name)
);
CREATE INDEX idx_draft_dataframes_artifact ON draft_dataframes(artifact_id);
CREATE TABLE model_dataframes (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 version_id INTEGER NOT NULL REFERENCES model_versions(id) ON DELETE CASCADE,
 table_name TEXT NOT NULL CHECK(length(table_name)>0 AND lower(table_name)<>'para'),
 artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
 schema_json TEXT NOT NULL,
 row_count INTEGER NOT NULL CHECK(row_count>=0),
 content_hash TEXT NOT NULL CHECK(length(content_hash)=64),
 created_at_ms INTEGER NOT NULL,
 UNIQUE(version_id,table_name)
);
CREATE INDEX idx_model_dataframes_artifact ON model_dataframes(artifact_id);

CREATE TABLE upload_batches (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 client_batch_id TEXT NOT NULL,
 source_kind TEXT NOT NULL CHECK(source_kind IN ('upload','case')),
 replacement_resource_id TEXT REFERENCES data_resources(id) ON DELETE RESTRICT,
 confirm_operation_id TEXT,
 confirm_payload_hash TEXT CHECK(confirm_payload_hash IS NULL OR length(confirm_payload_hash)=64),
 confirmation_started BOOLEAN NOT NULL DEFAULT 0,
 confirmation_in_progress BOOLEAN NOT NULL DEFAULT 0,
 status TEXT NOT NULL CHECK(status IN ('processing','awaiting_parse_settings','awaiting_purpose','ready_to_confirm','partially_confirmed','confirmed','failed','discarded','expired')),
 model_publish_status TEXT NOT NULL DEFAULT 'not_applicable' CHECK(model_publish_status IN ('not_applicable','ready','published','failed')),
 resource_publish_status TEXT NOT NULL DEFAULT 'not_applicable' CHECK(resource_publish_status IN ('not_applicable','ready','published','failed')),
 model_receipt_json TEXT,
 resource_receipt_json TEXT,
 model_diagnostics_json TEXT NOT NULL DEFAULT '[]',
 resource_diagnostics_json TEXT NOT NULL DEFAULT '[]',
 parse_settings_json TEXT NOT NULL DEFAULT '{}',
 diagnostics_json TEXT NOT NULL DEFAULT '[]',
 reserved_bytes INTEGER NOT NULL DEFAULT 0 CHECK(reserved_bytes>=0),
 expires_at_ms INTEGER NOT NULL,
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>0),
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 UNIQUE(session_id,client_batch_id)
);
CREATE INDEX idx_upload_batches_session_time ON upload_batches(session_id,created_at_ms DESC);
CREATE TRIGGER upload_batch_revision AFTER UPDATE ON upload_batches
WHEN NEW.revision=OLD.revision
BEGIN UPDATE upload_batches SET revision=OLD.revision+1 WHERE id=NEW.id; END;
CREATE TABLE upload_files (
 id TEXT PRIMARY KEY,
 batch_id INTEGER NOT NULL REFERENCES upload_batches(id) ON DELETE CASCADE,
 client_file_id TEXT NOT NULL,
 file_name TEXT NOT NULL,
 relative_path TEXT,
 media_type TEXT,
 staged_path TEXT NOT NULL,
 size_bytes INTEGER NOT NULL CHECK(size_bytes>=0),
 sha256 TEXT NOT NULL CHECK(length(sha256)=64),
 source_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE RESTRICT,
 created_at_ms INTEGER NOT NULL,
 UNIQUE(batch_id,client_file_id)
);
CREATE TABLE upload_candidates (
 id TEXT PRIMARY KEY,
 batch_id INTEGER NOT NULL REFERENCES upload_batches(id) ON DELETE CASCADE,
 file_id TEXT NOT NULL REFERENCES upload_files(id) ON DELETE CASCADE,
 sheet_name TEXT,
 display_name TEXT NOT NULL,
 proposed_alias TEXT,
 normalized_alias TEXT,
 alias_explicit BOOLEAN NOT NULL DEFAULT 0,
 purpose TEXT CHECK(purpose IN ('model_table','data_resource','skip')),
 inferred_schema_ref TEXT,
 normalized_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE RESTRICT,
 schema_json TEXT,
 schema_hash TEXT,
 row_count INTEGER CHECK(row_count IS NULL OR row_count>=0),
 evidence_json TEXT NOT NULL DEFAULT '{}',
 selected BOOLEAN NOT NULL DEFAULT 1,
 created_at_ms INTEGER NOT NULL
);

CREATE TABLE data_resources (
 id TEXT PRIMARY KEY,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 display_name TEXT NOT NULL CHECK(length(trim(display_name))>0),
 alias TEXT NOT NULL,
 normalized_alias TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('live','replaced','deleted','needs_rename')),
 declared_schema_ref TEXT,
 source_kind TEXT NOT NULL CHECK(source_kind IN ('upload','static_import','result','analysis','mapping','struct_edit')),
 normalized_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE RESTRICT,
 source_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE RESTRICT,
 schema_json TEXT NOT NULL,
 schema_hash TEXT NOT NULL CHECK(length(schema_hash)=64),
 row_count INTEGER NOT NULL CHECK(row_count>=0),
 source_batch_id INTEGER REFERENCES upload_batches(id) ON DELETE SET NULL,
 source_model_version_id INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
 replaced_by_resource_id TEXT REFERENCES data_resources(id) ON DELETE SET NULL,
 lineage_json TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 deleted_at_ms INTEGER,
 tombstone_json TEXT,
 CHECK((status='deleted')=(deleted_at_ms IS NOT NULL)),
 CHECK((status='deleted')=(normalized_artifact_id IS NULL)),
 CHECK(status!='deleted' OR source_artifact_id IS NULL)
);
CREATE UNIQUE INDEX idx_live_data_resource_alias
 ON data_resources(session_id,normalized_alias) WHERE status IN ('live','needs_rename');
CREATE INDEX idx_data_resources_session_time ON data_resources(session_id,created_at_ms DESC);

CREATE TABLE data_resource_deletion_previews (
 token_hash TEXT PRIMARY KEY CHECK(length(token_hash)=64),
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 resource_id TEXT NOT NULL REFERENCES data_resources(id) ON DELETE CASCADE,
 impact_hash TEXT NOT NULL CHECK(length(impact_hash)=64),
 impact_json TEXT NOT NULL,
 expires_at_ms INTEGER NOT NULL,
 created_at_ms INTEGER NOT NULL
);
CREATE TABLE data_resource_replacement_previews (
 token_hash TEXT PRIMARY KEY CHECK(length(token_hash)=64),
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 old_resource_id TEXT NOT NULL REFERENCES data_resources(id) ON DELETE CASCADE,
 pending_artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
 pending_resource_id TEXT NOT NULL,
 expected_content_hash TEXT NOT NULL CHECK(length(expected_content_hash)=64),
 schema_ref TEXT NOT NULL,
 schema_json TEXT NOT NULL,
 schema_hash TEXT NOT NULL CHECK(length(schema_hash)=64),
 values_json TEXT NOT NULL,
 display_name TEXT NOT NULL,
 proposed_alias TEXT NOT NULL,
 impact_hash TEXT NOT NULL CHECK(length(impact_hash)=64),
 impact_json TEXT NOT NULL,
 expires_at_ms INTEGER NOT NULL,
 created_at_ms INTEGER NOT NULL
);
CREATE TABLE publish_receipts (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 draft_id INTEGER NOT NULL,
 draft_revision INTEGER NOT NULL,
 client_publish_id TEXT NOT NULL,
 version_id INTEGER NOT NULL REFERENCES model_versions(id) ON DELETE RESTRICT,
 created_at_ms INTEGER NOT NULL,
 UNIQUE(draft_id,draft_revision,client_publish_id)
);

CREATE TABLE study_configurations (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 facade_id TEXT NOT NULL,
 facade_hash TEXT NOT NULL CHECK(length(facade_hash)=64),
 engine_snapshot_id TEXT NOT NULL REFERENCES engine_resource_snapshots(id) ON DELETE RESTRICT,
 status TEXT NOT NULL CHECK(status IN ('choosing_facade','collecting_inputs','ready_to_confirm','confirmed','invalidated','cancelled')),
 revision INTEGER NOT NULL CHECK(revision>0),
 intent_json TEXT NOT NULL,
 confirmed_at_ms INTEGER,
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL
);
CREATE TABLE study_input_bindings (
 configuration_id INTEGER NOT NULL REFERENCES study_configurations(id) ON DELETE CASCADE,
 input_name TEXT NOT NULL,
 input_kind TEXT NOT NULL CHECK(input_kind IN ('model','data','struct')),
 model_table_name TEXT,
 resource_id TEXT REFERENCES data_resources(id) ON DELETE RESTRICT,
 mapping_json TEXT,
 ordinal INTEGER NOT NULL,
 PRIMARY KEY(configuration_id,input_name),
 UNIQUE(configuration_id,ordinal)
);
CREATE TABLE study_struct_values (
 configuration_id INTEGER NOT NULL REFERENCES study_configurations(id) ON DELETE CASCADE,
 input_name TEXT NOT NULL,
 values_json TEXT NOT NULL,
 schema_ref TEXT NOT NULL,
 PRIMARY KEY(configuration_id,input_name)
);
CREATE TABLE study_result_selections (
 configuration_id INTEGER NOT NULL REFERENCES study_configurations(id) ON DELETE CASCADE,
 result_name TEXT NOT NULL,
 ordinal INTEGER NOT NULL,
 PRIMARY KEY(configuration_id,result_name),
 UNIQUE(configuration_id,ordinal)
);

CREATE TABLE managed_workflows (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 configuration_id INTEGER REFERENCES study_configurations(id) ON DELETE SET NULL,
 configuration_revision INTEGER CHECK(configuration_revision IS NULL OR configuration_revision>0),
 source_workflow_id INTEGER REFERENCES managed_workflows(id) ON DELETE SET NULL,
 client_operation_id TEXT,
 status TEXT NOT NULL CHECK(status IN ('queued','generating','executing','repairing','succeeded','failed','cancelled_by_user','service_restarted')),
 intent_json TEXT NOT NULL,
 locked_contract_json TEXT,
 input_diff_json TEXT,
 engine_snapshot_id TEXT REFERENCES engine_resource_snapshots(id) ON DELETE RESTRICT,
 facade_contract_hash TEXT,
 correction_budget INTEGER NOT NULL DEFAULT 0 CHECK(correction_budget>=0),
 corrections_used INTEGER NOT NULL DEFAULT 0 CHECK(corrections_used>=0 AND corrections_used<=correction_budget),
 progress_message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
 final_script_resource_id INTEGER REFERENCES script_resources(id) ON DELETE SET NULL,
 final_run_id INTEGER REFERENCES simulation_runs(id) ON DELETE SET NULL,
 error_json TEXT,
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 finished_at_ms INTEGER
);
CREATE UNIQUE INDEX idx_one_nonterminal_workflow_per_session ON managed_workflows(session_id)
 WHERE status IN ('queued','generating','executing','repairing');
CREATE UNIQUE INDEX idx_managed_workflow_operation ON managed_workflows(session_id,client_operation_id)
 WHERE client_operation_id IS NOT NULL;
CREATE TABLE workflow_input_snapshots (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 workflow_id INTEGER NOT NULL REFERENCES managed_workflows(id) ON DELETE CASCADE,
 input_name TEXT NOT NULL,
 input_kind TEXT NOT NULL,
 resource_id TEXT REFERENCES data_resources(id) ON DELETE SET NULL,
 model_version_id INTEGER REFERENCES model_versions(id) ON DELETE RESTRICT,
 artifact_id INTEGER REFERENCES artifacts(id) ON DELETE RESTRICT,
 input_alias TEXT,
 schema_ref TEXT,
 schema_hash TEXT,
 content_hash TEXT,
 snapshot_json TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 UNIQUE(workflow_id,input_name)
);
CREATE TABLE workflow_attempts (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 workflow_id INTEGER NOT NULL REFERENCES managed_workflows(id) ON DELETE CASCADE,
 attempt_number INTEGER NOT NULL CHECK(attempt_number>0),
 parent_attempt_id INTEGER REFERENCES workflow_attempts(id) ON DELETE SET NULL,
 phase TEXT NOT NULL CHECK(phase IN ('generation','repair')),
 script_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE RESTRICT,
 script_content_hash TEXT,
 status TEXT NOT NULL CHECK(status IN ('candidate','contract_rejected','executing','engine_failed','result_rejected','succeeded')),
 exit_code INTEGER,
 stderr_audit_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE RESTRICT,
 stdout_audit_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE RESTRICT,
 sanitized_stderr TEXT,
 stderr_full_hash TEXT,
 stderr_retained_hash TEXT,
 stderr_sent_hash TEXT,
 stderr_total_bytes INTEGER CHECK(stderr_total_bytes IS NULL OR stderr_total_bytes>=0),
 stderr_sent_bytes INTEGER CHECK(stderr_sent_bytes IS NULL OR stderr_sent_bytes>=0),
 stderr_truncated BOOLEAN NOT NULL DEFAULT 0,
 stdout_full_hash TEXT,
 stdout_retained_hash TEXT,
 stdout_total_bytes INTEGER CHECK(stdout_total_bytes IS NULL OR stdout_total_bytes>=0),
 stdout_truncated BOOLEAN NOT NULL DEFAULT 0,
 result_diagnostics_json TEXT,
 agent_bundle_version TEXT,
 agent_skill_refs_json TEXT NOT NULL DEFAULT '[]',
 agent_model TEXT,
 engine_snapshot_id TEXT REFERENCES engine_resource_snapshots(id) ON DELETE RESTRICT,
 created_at_ms INTEGER NOT NULL,
 started_at_ms INTEGER,
 finished_at_ms INTEGER,
 UNIQUE(workflow_id,attempt_number)
);
CREATE TABLE retry_previews (
 token_hash TEXT PRIMARY KEY CHECK(length(token_hash)=64),
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 workflow_id INTEGER NOT NULL REFERENCES managed_workflows(id) ON DELETE CASCADE,
 impact_hash TEXT NOT NULL CHECK(length(impact_hash)=64),
 impact_json TEXT NOT NULL,
 expires_at_ms INTEGER NOT NULL,
 created_at_ms INTEGER NOT NULL
);

CREATE TABLE script_resources (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
 name TEXT NOT NULL CHECK(length(name)>0),
 source_kind TEXT NOT NULL CHECK(source_kind IN ('upload','agent','manual')),
 content_hash TEXT NOT NULL CHECK(length(content_hash)=64),
 validation_status TEXT NOT NULL CHECK(validation_status IN ('valid','invalid')),
 required_aliases_json TEXT NOT NULL,
 outputs_json TEXT NOT NULL,
 diagnostics_json TEXT NOT NULL,
 agent_bundle_version TEXT,
 agent_skill_refs_json TEXT NOT NULL,
 managed_workflow_id INTEGER REFERENCES managed_workflows(id) ON DELETE SET NULL,
 source_script_resource_id INTEGER REFERENCES script_resources(id) ON DELETE SET NULL,
 frozen_contract_json TEXT,
 engine_snapshot_id TEXT REFERENCES engine_resource_snapshots(id) ON DELETE RESTRICT,
 facade_contract_hash TEXT,
 created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_scripts_session_time ON script_resources(session_id,created_at_ms,id);

CREATE TABLE simulation_runs (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 model_version_id INTEGER REFERENCES model_versions(id) ON DELETE RESTRICT,
 model_content_hash TEXT CHECK(model_content_hash IS NULL OR length(model_content_hash)=64),
 script_resource_id INTEGER REFERENCES script_resources(id) ON DELETE RESTRICT,
 script_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE RESTRICT,
 script_content_hash TEXT CHECK(script_content_hash IS NULL OR length(script_content_hash)=64),
 client_run_id TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','failed')),
 stage TEXT NOT NULL,
 engine_contract_version TEXT NOT NULL,
 agent_bundle_version TEXT,
 agent_skill_refs_json TEXT NOT NULL,
 script_message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
 managed_workflow_id INTEGER REFERENCES managed_workflows(id) ON DELETE SET NULL,
 run_kind TEXT NOT NULL DEFAULT 'manual' CHECK(run_kind IN ('managed','agent_rerun','manual')),
 input_snapshot_json TEXT NOT NULL DEFAULT '{}',
 result_contract_json TEXT,
 exit_code INTEGER,
 error_json TEXT,
 created_at_ms INTEGER NOT NULL,
 started_at_ms INTEGER,
 finished_at_ms INTEGER,
 deleted_at_ms INTEGER,
 tombstone_json TEXT,
 CHECK((script_resource_id IS NULL AND script_artifact_id IS NULL AND script_content_hash IS NULL
        AND run_kind='managed')
    OR (script_resource_id IS NOT NULL AND script_artifact_id IS NOT NULL AND script_content_hash IS NOT NULL)),
 UNIQUE(session_id,client_run_id)
);
CREATE INDEX idx_runs_session_time ON simulation_runs(session_id,created_at_ms,id);
CREATE INDEX idx_runs_status ON simulation_runs(status);
CREATE INDEX idx_runs_model_version ON simulation_runs(model_version_id);

CREATE TABLE messages (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 run_id INTEGER REFERENCES simulation_runs(id) ON DELETE SET NULL,
 operation_id TEXT,
 revision INTEGER NOT NULL DEFAULT 1 CHECK(revision>0),
 role TEXT NOT NULL,
 kind TEXT NOT NULL,
 content TEXT NOT NULL DEFAULT '',
 payload_json TEXT,
 created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_messages_session_time ON messages(session_id,created_at_ms);
CREATE INDEX idx_messages_run ON messages(run_id);
CREATE UNIQUE INDEX uq_messages_operation ON messages(session_id,kind,operation_id)
 WHERE operation_id IS NOT NULL;

CREATE TABLE result_analysis_runs (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 request_message_id INTEGER REFERENCES messages(id) ON DELETE SET NULL,
 intent_json TEXT NOT NULL,
 script_artifact_id INTEGER NOT NULL REFERENCES artifacts(id),
 status TEXT NOT NULL CHECK(status IN ('pending','running','succeeded','failed')),
 stage TEXT NOT NULL,
 error_json TEXT,
 agent_bundle_version TEXT NOT NULL,
 agent_skill_refs_json TEXT NOT NULL,
 engine_contract_version TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 started_at_ms INTEGER,
 finished_at_ms INTEGER
);
CREATE INDEX idx_result_analysis_runs_session ON result_analysis_runs(session_id,id);
CREATE TABLE result_datasets (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 run_id INTEGER REFERENCES simulation_runs(id) ON DELETE CASCADE,
 analysis_run_id INTEGER REFERENCES result_analysis_runs(id) ON DELETE CASCADE,
 artifact_id INTEGER REFERENCES artifacts(id),
 result_name TEXT NOT NULL,
 schema_json TEXT NOT NULL,
 schema_hash TEXT NOT NULL,
 row_count INTEGER NOT NULL CHECK(row_count>=0),
 size_bytes INTEGER NOT NULL CHECK(size_bytes>=0),
 created_at_ms INTEGER NOT NULL,
 deleted_at_ms INTEGER,
 tombstone_json TEXT,
 CHECK((run_id IS NOT NULL)<>(analysis_run_id IS NOT NULL)),
 CHECK((deleted_at_ms IS NULL AND artifact_id IS NOT NULL AND tombstone_json IS NULL)
    OR (deleted_at_ms IS NOT NULL AND artifact_id IS NULL AND tombstone_json IS NOT NULL)),
 UNIQUE(run_id,result_name),
 UNIQUE(analysis_run_id,result_name)
);
CREATE INDEX idx_result_datasets_run ON result_datasets(run_id,id);
CREATE INDEX idx_result_datasets_analysis ON result_datasets(analysis_run_id,id);
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
 CHECK((dataset_id IS NOT NULL)<>(resource_id IS NOT NULL)),
 UNIQUE(analysis_run_id,dataset_id),
 UNIQUE(analysis_run_id,resource_id),
 UNIQUE(analysis_run_id,alias),
 UNIQUE(analysis_run_id,ordinal)
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
 CHECK((source_dataset_id IS NOT NULL)<>(source_resource_id IS NOT NULL)),
 UNIQUE(derived_dataset_id,source_dataset_id),
 UNIQUE(derived_dataset_id,source_resource_id)
);
CREATE TABLE dataset_views (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 source_dataset_id INTEGER NOT NULL REFERENCES result_datasets(id) ON DELETE RESTRICT,
 view_dataset_id INTEGER NOT NULL UNIQUE REFERENCES result_datasets(id) ON DELETE RESTRICT,
 name TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('line','bar','scatter','heatmap','boxplot','histogram')),
 spec_json TEXT NOT NULL,
 is_default BOOLEAN NOT NULL DEFAULT 0,
 created_at_ms INTEGER NOT NULL,
 UNIQUE(source_dataset_id,name)
);
CREATE INDEX idx_dataset_views_source ON dataset_views(source_dataset_id,id);
CREATE TABLE run_comparisons (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 baseline_run_id INTEGER NOT NULL REFERENCES simulation_runs(id) ON DELETE RESTRICT,
 candidate_run_id INTEGER NOT NULL REFERENCES simulation_runs(id) ON DELETE RESTRICT,
 baseline_snapshot_json TEXT NOT NULL,
 candidate_snapshot_json TEXT NOT NULL,
 compatibility_json TEXT NOT NULL,
 analysis_run_id INTEGER NOT NULL UNIQUE REFERENCES result_analysis_runs(id) ON DELETE RESTRICT,
 created_at_ms INTEGER NOT NULL,
 CHECK(baseline_run_id<>candidate_run_id)
);
CREATE INDEX idx_comparisons_session_time ON run_comparisons(session_id,created_at_ms,id);

CREATE TABLE version_deletion_previews (
 token_hash TEXT PRIMARY KEY CHECK(length(token_hash)=64),
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 version_id INTEGER NOT NULL REFERENCES model_versions(id) ON DELETE CASCADE,
 impact_hash TEXT NOT NULL CHECK(length(impact_hash)=64),
 impact_json TEXT NOT NULL,
 expires_at_ms INTEGER NOT NULL,
 created_at_ms INTEGER NOT NULL
);

CREATE TABLE session_deletion_previews (
 token_hash TEXT PRIMARY KEY CHECK(length(token_hash)=64),
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 impact_hash TEXT NOT NULL CHECK(length(impact_hash)=64),
 impact_json TEXT NOT NULL,
 expires_at_ms INTEGER NOT NULL,
 created_at_ms INTEGER NOT NULL
);
CREATE TABLE run_deletion_previews (
 token_hash TEXT PRIMARY KEY CHECK(length(token_hash)=64),
 user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
 run_id INTEGER NOT NULL REFERENCES simulation_runs(id) ON DELETE CASCADE,
 impact_hash TEXT NOT NULL CHECK(length(impact_hash)=64),
 impact_json TEXT NOT NULL,
 expires_at_ms INTEGER NOT NULL,
 created_at_ms INTEGER NOT NULL
);

CREATE TABLE chat_search_documents (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 source_kind TEXT NOT NULL CHECK(source_kind IN ('session_title','message','upload_batch','run_summary','script')),
 source_id INTEGER,
 search_text TEXT NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 UNIQUE(session_id,source_kind,source_id)
);
CREATE INDEX idx_chat_search_session_source ON chat_search_documents(session_id,source_kind,source_id);
CREATE VIRTUAL TABLE chat_search_fts USING fts5(search_text,content='chat_search_documents',content_rowid='id',tokenize='trigram');

CREATE TRIGGER validate_workspace_active_update BEFORE UPDATE OF active_version_id ON workspaces WHEN NEW.active_version_id IS NOT NULL BEGIN
 SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM model_versions WHERE id=NEW.active_version_id AND workspace_id=NEW.id AND deleted_at_ms IS NULL) THEN RAISE(ABORT,'invalid active model version') END;
END;
CREATE TRIGGER live_session_messages_insert BEFORE INSERT ON messages WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_messages_update BEFORE UPDATE ON messages WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') AND NOT(OLD.run_id IS NOT NULL AND NEW.run_id IS NULL) BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_workspaces_insert BEFORE INSERT ON workspaces WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_workspace_drafts_insert BEFORE INSERT ON workspace_drafts WHEN NOT EXISTS(SELECT 1 FROM workspaces w JOIN sessions s ON s.id=w.session_id WHERE w.id=NEW.workspace_id AND s.deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_workspace_drafts_update BEFORE UPDATE ON workspace_drafts WHEN NOT EXISTS(SELECT 1 FROM workspaces w JOIN sessions s ON s.id=w.session_id WHERE w.id=NEW.workspace_id AND s.deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_draft_dataframes_insert BEFORE INSERT ON draft_dataframes WHEN NOT EXISTS(SELECT 1 FROM workspace_drafts wd JOIN workspaces w ON w.id=wd.workspace_id JOIN sessions s ON s.id=w.session_id WHERE wd.id=NEW.draft_id AND s.deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_draft_dataframes_update BEFORE UPDATE ON draft_dataframes WHEN NOT EXISTS(SELECT 1 FROM workspace_drafts wd JOIN workspaces w ON w.id=wd.workspace_id JOIN sessions s ON s.id=w.session_id WHERE wd.id=NEW.draft_id AND s.deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_workspaces_update BEFORE UPDATE ON workspaces
WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live')
 AND NOT (
   NEW.session_id=OLD.session_id
   AND NEW.active_version_id IS NULL
   AND OLD.active_version_id IS NOT NULL
   AND NEW.created_at_ms=OLD.created_at_ms
   AND NEW.updated_at_ms=OLD.updated_at_ms
   AND EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='deleting')
 )
BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_uploads_insert BEFORE INSERT ON upload_batches WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_uploads_update BEFORE UPDATE ON upload_batches WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_resources_insert BEFORE INSERT ON data_resources WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_resources_update BEFORE UPDATE ON data_resources WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_configurations_insert BEFORE INSERT ON study_configurations WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_configurations_update BEFORE UPDATE ON study_configurations WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_scripts_insert BEFORE INSERT ON script_resources WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_runs_insert BEFORE INSERT ON simulation_runs WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_runs_update BEFORE UPDATE ON simulation_runs WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_analysis_insert BEFORE INSERT ON result_analysis_runs WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_analysis_update BEFORE UPDATE ON result_analysis_runs WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_workflows_insert BEFORE INSERT ON managed_workflows WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_workflows_update BEFORE UPDATE ON managed_workflows WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER live_session_comparisons_insert BEFORE INSERT ON run_comparisons WHEN NOT EXISTS(SELECT 1 FROM sessions WHERE id=NEW.session_id AND deletion_status='live') BEGIN SELECT RAISE(ABORT,'session is not live'); END;
CREATE TRIGGER immutable_model_versions BEFORE UPDATE OF workspace_id,description,content_hash,source_summary_json,engine_contract_version,created_at_ms ON model_versions BEGIN SELECT RAISE(ABORT,'model versions are immutable'); END;
CREATE TRIGGER immutable_model_dataframes BEFORE UPDATE ON model_dataframes BEGIN SELECT RAISE(ABORT,'model dataframes are immutable'); END;
CREATE TRIGGER immutable_workflow_inputs BEFORE UPDATE ON workflow_input_snapshots BEGIN SELECT RAISE(ABORT,'workflow inputs are immutable'); END;
CREATE TRIGGER immutable_workflow_attempt_identity BEFORE UPDATE OF workflow_id,attempt_number,parent_attempt_id,phase,script_artifact_id,script_content_hash,agent_bundle_version,agent_skill_refs_json,agent_model,engine_snapshot_id,created_at_ms ON workflow_attempts BEGIN SELECT RAISE(ABORT,'workflow attempt candidate is immutable'); END;
CREATE TRIGGER validate_run_insert BEFORE INSERT ON simulation_runs BEGIN
 SELECT CASE WHEN NEW.script_resource_id IS NOT NULL AND NOT EXISTS(
   SELECT 1 FROM sessions s
   JOIN script_resources r ON r.session_id=s.id
   WHERE s.id=NEW.session_id AND r.id=NEW.script_resource_id
 ) THEN RAISE(ABORT,'run resources must belong to one session') END;
 SELECT CASE WHEN (NEW.model_version_id IS NULL)<>(NEW.model_content_hash IS NULL)
   THEN RAISE(ABORT,'run model id and hash must both be null or present') END;
 SELECT CASE WHEN NEW.model_version_id IS NOT NULL AND NOT EXISTS(
   SELECT 1 FROM workspaces w JOIN model_versions v ON v.workspace_id=w.id
   WHERE w.session_id=NEW.session_id AND v.id=NEW.model_version_id
     AND v.content_hash=NEW.model_content_hash AND v.deleted_at_ms IS NULL
 ) THEN RAISE(ABORT,'run model must belong to session') END;
END;
CREATE TRIGGER validate_run_script_binding_update BEFORE UPDATE OF script_resource_id,script_artifact_id,script_content_hash ON simulation_runs BEGIN
 SELECT CASE WHEN OLD.script_resource_id IS NOT NULL AND
   (NEW.script_resource_id IS NOT OLD.script_resource_id OR NEW.script_artifact_id IS NOT OLD.script_artifact_id OR NEW.script_content_hash IS NOT OLD.script_content_hash)
   THEN RAISE(ABORT,'run script binding is write once') END;
 SELECT CASE WHEN NEW.script_resource_id IS NULL OR NEW.script_artifact_id IS NULL OR NEW.script_content_hash IS NULL
   THEN RAISE(ABORT,'run script binding cannot be cleared') END;
 SELECT CASE WHEN NOT EXISTS(
   SELECT 1 FROM script_resources r WHERE r.id=NEW.script_resource_id AND r.session_id=NEW.session_id
     AND r.artifact_id=NEW.script_artifact_id AND r.content_hash=NEW.script_content_hash
 ) THEN RAISE(ABORT,'run resources must belong to one session') END;
END;
CREATE TRIGGER immutable_data_resource_payload BEFORE UPDATE OF normalized_artifact_id,source_artifact_id,schema_json,schema_hash,row_count,source_kind,source_batch_id,lineage_json ON data_resources
WHEN NOT (
 OLD.status!='deleted' AND NEW.status='deleted'
 AND NEW.normalized_artifact_id IS NULL AND NEW.source_artifact_id IS NULL
 AND NEW.schema_json IS OLD.schema_json AND NEW.schema_hash IS OLD.schema_hash
 AND NEW.row_count IS OLD.row_count AND NEW.source_kind IS OLD.source_kind
 AND NEW.source_batch_id IS OLD.source_batch_id AND NEW.lineage_json IS OLD.lineage_json
)
BEGIN
 SELECT RAISE(ABORT,'data resource payload is immutable');
END;
CREATE TRIGGER validate_run_script_message_insert BEFORE INSERT ON simulation_runs WHEN NEW.script_message_id IS NOT NULL BEGIN
 SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM messages m WHERE m.id=NEW.script_message_id AND m.session_id=NEW.session_id AND m.kind='script')
   THEN RAISE(ABORT,'script message must belong to run session') END;
END;
CREATE TRIGGER validate_run_script_message_update BEFORE UPDATE OF script_message_id ON simulation_runs BEGIN
 SELECT CASE WHEN OLD.script_message_id IS NOT NULL AND NEW.script_message_id IS NOT OLD.script_message_id
   THEN RAISE(ABORT,'script message anchor is write once') END;
 SELECT CASE WHEN NEW.script_message_id IS NOT NULL AND NOT EXISTS(SELECT 1 FROM messages m WHERE m.id=NEW.script_message_id AND m.session_id=NEW.session_id AND m.kind='script')
   THEN RAISE(ABORT,'script message must belong to run session') END;
END;
CREATE TRIGGER tombstone_run_results AFTER UPDATE OF deleted_at_ms ON simulation_runs WHEN OLD.deleted_at_ms IS NULL AND NEW.deleted_at_ms IS NOT NULL BEGIN
 UPDATE result_datasets SET artifact_id=NULL,deleted_at_ms=NEW.deleted_at_ms,
   tombstone_json=COALESCE(tombstone_json,json_object('resultName',result_name,'runId',NEW.id))
 WHERE run_id=NEW.id AND deleted_at_ms IS NULL;
END;

CREATE TRIGGER chat_docs_ai AFTER INSERT ON chat_search_documents BEGIN INSERT INTO chat_search_fts(rowid,search_text) VALUES(new.id,new.search_text); END;
CREATE TRIGGER chat_docs_ad AFTER DELETE ON chat_search_documents BEGIN INSERT INTO chat_search_fts(chat_search_fts,rowid,search_text) VALUES('delete',old.id,old.search_text); END;
CREATE TRIGGER chat_docs_au AFTER UPDATE ON chat_search_documents BEGIN
 INSERT INTO chat_search_fts(chat_search_fts,rowid,search_text) VALUES('delete',old.id,old.search_text);
 INSERT INTO chat_search_fts(rowid,search_text) VALUES(new.id,new.search_text);
END;
CREATE TRIGGER chat_session_ai AFTER INSERT ON sessions BEGIN INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) VALUES(new.id,'session_title',new.id,new.title,new.updated_at_ms); END;
CREATE TRIGGER chat_session_au AFTER UPDATE OF title ON sessions BEGIN UPDATE chat_search_documents SET search_text=new.title,updated_at_ms=new.updated_at_ms WHERE session_id=new.id AND source_kind='session_title' AND source_id=new.id; END;
CREATE TRIGGER chat_message_ai AFTER INSERT ON messages WHEN new.role IN ('user','assistant') OR new.kind IN ('upload-batch','study-config','workflow-progress') BEGIN INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) VALUES(new.session_id,'message',new.id,new.content,new.created_at_ms); END;
CREATE TRIGGER chat_message_au AFTER UPDATE OF content,revision ON messages WHEN new.role IN ('user','assistant') OR new.kind IN ('upload-batch','study-config','workflow-progress') BEGIN UPDATE chat_search_documents SET search_text=new.content,updated_at_ms=new.created_at_ms WHERE session_id=new.session_id AND source_kind='message' AND source_id=new.id; END;
CREATE TRIGGER chat_message_ad AFTER DELETE ON messages BEGIN DELETE FROM chat_search_documents WHERE session_id=old.session_id AND source_kind='message' AND source_id=old.id; END;
CREATE TRIGGER chat_upload_ai AFTER INSERT ON upload_batches BEGIN
 INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms)
 VALUES(new.session_id,'upload_batch',NULL,new.client_batch_id,new.created_at_ms);
END;
CREATE TRIGGER chat_script_ai AFTER INSERT ON script_resources BEGIN
 INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms)
 VALUES(new.session_id,'script',new.id,new.name,new.created_at_ms);
END;
CREATE TRIGGER chat_run_ai AFTER INSERT ON simulation_runs BEGIN
 INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms)
 SELECT new.session_id,'run_summary',new.id,s.name||' '||new.status,new.created_at_ms
 FROM script_resources s WHERE s.id=new.script_resource_id;
END;
CREATE TRIGGER chat_run_au AFTER UPDATE OF status ON simulation_runs BEGIN
 UPDATE chat_search_documents SET
   search_text=(SELECT name FROM script_resources WHERE id=new.script_resource_id)||' '||new.status,
   updated_at_ms=COALESCE(new.finished_at_ms,new.started_at_ms,new.created_at_ms)
 WHERE session_id=new.session_id AND source_kind='run_summary' AND source_id=new.id;
END;
