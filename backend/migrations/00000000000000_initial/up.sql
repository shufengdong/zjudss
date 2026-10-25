-- zjudss v3 initial schema. Earlier versions were never deployed; only empty-database initialization is supported.

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
 title_source TEXT NOT NULL DEFAULT 'default' CHECK(title_source IN ('default','auto','user'))
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
 table_name TEXT NOT NULL CHECK(length(table_name)>0),
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
 table_name TEXT NOT NULL CHECK(length(table_name)>0),
 artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
 schema_json TEXT NOT NULL,
 row_count INTEGER NOT NULL CHECK(row_count>=0),
 content_hash TEXT NOT NULL CHECK(length(content_hash)=64),
 created_at_ms INTEGER NOT NULL,
 UNIQUE(version_id,table_name)
);
CREATE INDEX idx_model_dataframes_artifact ON model_dataframes(artifact_id);

CREATE TABLE model_imports (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 client_import_id TEXT NOT NULL,
 source_kind TEXT NOT NULL CHECK(source_kind IN ('upload','case')),
 source_summary_json TEXT NOT NULL,
 source_sha256 TEXT NOT NULL CHECK(length(source_sha256)=64),
 base_draft_id INTEGER REFERENCES workspace_drafts(id) ON DELETE SET NULL,
 base_draft_revision INTEGER,
 status TEXT NOT NULL CHECK(status IN ('accepted','processing','awaiting_resolution','ready_to_apply','applied','failed','discarded')),
 stages_json TEXT NOT NULL,
 diagnostics_json TEXT NOT NULL,
 error_code TEXT,
 error_message TEXT,
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 UNIQUE(session_id,client_import_id),
 CHECK((base_draft_id IS NULL)=(base_draft_revision IS NULL))
);
CREATE INDEX idx_model_imports_workspace_time ON model_imports(workspace_id,created_at_ms DESC,id DESC);
CREATE TABLE model_import_candidates (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 import_id INTEGER NOT NULL REFERENCES model_imports(id) ON DELETE CASCADE,
 candidate_id TEXT NOT NULL,
 candidate_kind TEXT NOT NULL CHECK(candidate_kind IN ('entry','table')),
 display_name TEXT NOT NULL,
 table_name TEXT,
 artifact_id INTEGER REFERENCES artifacts(id) ON DELETE RESTRICT,
 schema_json TEXT,
 row_count INTEGER CHECK(row_count IS NULL OR row_count>=0),
 content_hash TEXT,
 evidence_json TEXT NOT NULL,
 selected BOOLEAN NOT NULL DEFAULT 0,
 created_at_ms INTEGER NOT NULL,
 UNIQUE(import_id,candidate_id)
);
CREATE INDEX idx_import_candidates_artifact ON model_import_candidates(artifact_id);
CREATE TABLE model_import_selections (
 import_id INTEGER NOT NULL REFERENCES model_imports(id) ON DELETE CASCADE,
 candidate_id TEXT NOT NULL,
 table_name TEXT,
 selected BOOLEAN NOT NULL,
 created_at_ms INTEGER NOT NULL,
 PRIMARY KEY(import_id,candidate_id),
 FOREIGN KEY(import_id,candidate_id)
   REFERENCES model_import_candidates(import_id,candidate_id) ON DELETE CASCADE
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
 created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_scripts_session_time ON script_resources(session_id,created_at_ms,id);

CREATE TABLE simulation_runs (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 model_version_id INTEGER NOT NULL REFERENCES model_versions(id) ON DELETE RESTRICT,
 model_content_hash TEXT NOT NULL CHECK(length(model_content_hash)=64),
 script_resource_id INTEGER NOT NULL REFERENCES script_resources(id) ON DELETE RESTRICT,
 script_artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
 script_content_hash TEXT NOT NULL CHECK(length(script_content_hash)=64),
 client_run_id TEXT NOT NULL,
 status TEXT NOT NULL CHECK(status IN ('queued','running','succeeded','failed')),
 stage TEXT NOT NULL,
 engine_contract_version TEXT NOT NULL,
 agent_bundle_version TEXT,
 agent_skill_refs_json TEXT NOT NULL,
 exit_code INTEGER,
 error_json TEXT,
 created_at_ms INTEGER NOT NULL,
 started_at_ms INTEGER,
 finished_at_ms INTEGER,
 deleted_at_ms INTEGER,
 tombstone_json TEXT,
 UNIQUE(session_id,client_run_id)
);
CREATE INDEX idx_runs_session_time ON simulation_runs(session_id,created_at_ms,id);
CREATE INDEX idx_runs_status ON simulation_runs(status);
CREATE INDEX idx_runs_model_version ON simulation_runs(model_version_id);

CREATE TABLE messages (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 run_id INTEGER REFERENCES simulation_runs(id) ON DELETE SET NULL,
 role TEXT NOT NULL,
 kind TEXT NOT NULL,
 content TEXT NOT NULL DEFAULT '',
 payload_json TEXT,
 created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_messages_session_time ON messages(session_id,created_at_ms);
CREATE INDEX idx_messages_run ON messages(run_id);

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
 analysis_run_id INTEGER NOT NULL REFERENCES result_analysis_runs(id) ON DELETE CASCADE,
 dataset_id INTEGER NOT NULL REFERENCES result_datasets(id) ON DELETE RESTRICT,
 alias TEXT NOT NULL,
 ordinal INTEGER NOT NULL,
 source_name TEXT NOT NULL,
 source_schema_json TEXT NOT NULL,
 source_schema_hash TEXT NOT NULL,
 source_producer_json TEXT NOT NULL,
 PRIMARY KEY(analysis_run_id,dataset_id),
 UNIQUE(analysis_run_id,alias),
 UNIQUE(analysis_run_id,ordinal)
);
CREATE TABLE dataset_lineage_snapshots (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 derived_dataset_id INTEGER NOT NULL REFERENCES result_datasets(id) ON DELETE CASCADE,
 source_dataset_id INTEGER NOT NULL REFERENCES result_datasets(id) ON DELETE RESTRICT,
 source_name TEXT NOT NULL,
 source_schema_json TEXT NOT NULL,
 source_schema_hash TEXT NOT NULL,
 source_producer_json TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 UNIQUE(derived_dataset_id,source_dataset_id)
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

CREATE TABLE chat_search_documents (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 source_kind TEXT NOT NULL CHECK(source_kind IN ('session_title','message','model_import','run_summary','script')),
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
CREATE TRIGGER immutable_model_versions BEFORE UPDATE OF workspace_id,content_hash,source_summary_json,engine_contract_version,created_at_ms ON model_versions BEGIN SELECT RAISE(ABORT,'model versions are immutable'); END;
CREATE TRIGGER immutable_model_dataframes BEFORE UPDATE ON model_dataframes BEGIN SELECT RAISE(ABORT,'model dataframes are immutable'); END;
CREATE TRIGGER validate_run_insert BEFORE INSERT ON simulation_runs BEGIN
 SELECT CASE WHEN NOT EXISTS(
   SELECT 1 FROM sessions s
   JOIN workspaces w ON w.session_id=s.id
   JOIN model_versions v ON v.workspace_id=w.id
   JOIN script_resources r ON r.session_id=s.id
   WHERE s.id=NEW.session_id AND v.id=NEW.model_version_id
     AND v.deleted_at_ms IS NULL AND r.id=NEW.script_resource_id
 ) THEN RAISE(ABORT,'run resources must belong to one session') END;
END;

CREATE TRIGGER chat_docs_ai AFTER INSERT ON chat_search_documents BEGIN INSERT INTO chat_search_fts(rowid,search_text) VALUES(new.id,new.search_text); END;
CREATE TRIGGER chat_docs_ad AFTER DELETE ON chat_search_documents BEGIN INSERT INTO chat_search_fts(chat_search_fts,rowid,search_text) VALUES('delete',old.id,old.search_text); END;
CREATE TRIGGER chat_docs_au AFTER UPDATE ON chat_search_documents BEGIN
 INSERT INTO chat_search_fts(chat_search_fts,rowid,search_text) VALUES('delete',old.id,old.search_text);
 INSERT INTO chat_search_fts(rowid,search_text) VALUES(new.id,new.search_text);
END;
CREATE TRIGGER chat_session_ai AFTER INSERT ON sessions BEGIN INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) VALUES(new.id,'session_title',new.id,new.title,new.updated_at_ms); END;
CREATE TRIGGER chat_session_au AFTER UPDATE OF title ON sessions BEGIN UPDATE chat_search_documents SET search_text=new.title,updated_at_ms=new.updated_at_ms WHERE session_id=new.id AND source_kind='session_title' AND source_id=new.id; END;
CREATE TRIGGER chat_message_ai AFTER INSERT ON messages WHEN new.role IN ('user','assistant') BEGIN INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) VALUES(new.session_id,'message',new.id,new.content,new.created_at_ms); END;
CREATE TRIGGER chat_message_ad AFTER DELETE ON messages BEGIN DELETE FROM chat_search_documents WHERE session_id=old.session_id AND source_kind='message' AND source_id=old.id; END;
CREATE TRIGGER chat_import_ai AFTER INSERT ON model_imports BEGIN
 INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms)
 VALUES(new.session_id,'model_import',new.id,new.source_summary_json,new.created_at_ms);
END;
CREATE TRIGGER chat_import_ad AFTER DELETE ON model_imports BEGIN DELETE FROM chat_search_documents WHERE source_kind='model_import' AND source_id=old.id; END;
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
