-- zjudss v2 initial schema. v1 was never deployed; only empty-database initialization is supported.

CREATE TABLE storage_scopes (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 kind TEXT NOT NULL CHECK(kind IN ('instance','owner','tenant')),
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
INSERT INTO storage_scopes(kind,scope_key,created_at_ms) VALUES('instance','instance',0);

CREATE TABLE sessions (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 title TEXT NOT NULL DEFAULT '',
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 access_scope_kind TEXT NOT NULL CHECK(access_scope_kind IN ('instance','owner','tenant')),
 access_scope_key TEXT NOT NULL CHECK(length(access_scope_key)>0),
 storage_scope_id INTEGER NOT NULL REFERENCES storage_scopes(id),
 title_source TEXT NOT NULL DEFAULT 'default' CHECK(title_source IN ('default','auto','user'))
);
CREATE INDEX idx_sessions_updated ON sessions(updated_at_ms);
CREATE INDEX idx_sessions_access_scope ON sessions(access_scope_kind,access_scope_key,updated_at_ms);
CREATE INDEX idx_sessions_storage_scope ON sessions(storage_scope_id);

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
 draft_version_id INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 CHECK(active_version_id IS NULL OR draft_version_id IS NULL OR active_version_id<>draft_version_id)
);
CREATE TABLE model_versions (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
 parent_version_id INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
 status TEXT NOT NULL CHECK(status IN ('collecting','ready','invalid')),
 source_kind TEXT NOT NULL CHECK(source_kind IN ('rss','rssbundle','dataframes','mixed')),
 origin_kind TEXT NOT NULL CHECK(origin_kind IN ('upload','example','generated')),
 manifest_json TEXT NOT NULL,
 patch_json TEXT,
 content_hash TEXT NOT NULL,
 created_at_ms INTEGER NOT NULL,
 UNIQUE(workspace_id,content_hash)
);
CREATE INDEX idx_versions_workspace_time ON model_versions(workspace_id,created_at_ms DESC,id DESC);
CREATE TABLE version_artifacts (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 version_id INTEGER NOT NULL REFERENCES model_versions(id) ON DELETE CASCADE,
 artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
 role TEXT NOT NULL CHECK(role IN ('entry','source','data','input','generated','manifest')),
 relative_path TEXT,
 display_name TEXT NOT NULL,
 source_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE SET NULL,
 sheet_name TEXT
);
CREATE UNIQUE INDEX idx_version_artifact_path ON version_artifacts(version_id,relative_path) WHERE relative_path IS NOT NULL;
CREATE TABLE dataframe_bindings (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 version_artifact_id INTEGER NOT NULL UNIQUE REFERENCES version_artifacts(id) ON DELETE CASCADE,
 table_kind TEXT,
 confidence REAL CHECK(confidence IS NULL OR (confidence>=0 AND confidence<=1)),
 resolution TEXT NOT NULL CHECK(resolution IN ('pending','inferred','user_confirmed','rejected')),
 columns_json TEXT NOT NULL,
 evidence_json TEXT NOT NULL
);

CREATE TABLE ingestions (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 workspace_id INTEGER NOT NULL REFERENCES workspaces(id) ON DELETE CASCADE,
 client_batch_id TEXT NOT NULL,
 origin_kind TEXT NOT NULL CHECK(origin_kind IN ('upload','example')),
 source_ref_json TEXT,
 base_version_id INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
 published_version_id INTEGER REFERENCES model_versions(id) ON DELETE SET NULL,
 status TEXT NOT NULL CHECK(status IN ('accepted','extracting','inferring','validating','succeeded','completed_with_errors','failed')),
 stages_json TEXT NOT NULL,
 error_code TEXT,
 error_message TEXT,
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 UNIQUE(workspace_id,client_batch_id)
);
CREATE TABLE ingestion_artifacts (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 ingestion_id INTEGER NOT NULL REFERENCES ingestions(id) ON DELETE CASCADE,
 client_file_id TEXT NOT NULL,
 artifact_id INTEGER NOT NULL REFERENCES artifacts(id) ON DELETE RESTRICT,
 file_name TEXT NOT NULL,
 relative_path TEXT,
 media_type TEXT,
 role TEXT NOT NULL CHECK(role IN ('uploaded','example','extracted','normalized')),
 source_artifact_id INTEGER REFERENCES artifacts(id) ON DELETE SET NULL,
 sheet_name TEXT,
 status TEXT NOT NULL CHECK(status IN ('accepted','processing','ready','failed')),
 error_code TEXT,
 error_message TEXT,
 created_at_ms INTEGER NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 UNIQUE(ingestion_id,client_file_id,role,sheet_name)
);
CREATE INDEX idx_ingestions_workspace_time ON ingestions(workspace_id,created_at_ms DESC,id DESC);
CREATE INDEX idx_ingestion_artifacts_ingestion ON ingestion_artifacts(ingestion_id,id);

CREATE TABLE simulation_tasks (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 status TEXT NOT NULL CHECK(status IN ('pending','running','succeeded','failed','cancelled')),
 stage TEXT,
 study_type TEXT NOT NULL,
 model_version_id INTEGER NOT NULL REFERENCES model_versions(id),
 study_spec_json TEXT NOT NULL,
 script_artifact_id INTEGER NOT NULL REFERENCES artifacts(id),
 agent_bundle_version TEXT NOT NULL,
 agent_skill_refs_json TEXT NOT NULL,
 engine_contract_version TEXT NOT NULL,
 exit_code INTEGER,
 error_json TEXT,
 created_at_ms INTEGER NOT NULL,
 started_at_ms INTEGER,
 finished_at_ms INTEGER
);
CREATE INDEX idx_tasks_session_time ON simulation_tasks(session_id,created_at_ms);
CREATE INDEX idx_tasks_session_id ON simulation_tasks(session_id,id);
CREATE INDEX idx_tasks_status ON simulation_tasks(status);
CREATE INDEX idx_simulation_tasks_model_version ON simulation_tasks(model_version_id);

CREATE TABLE messages (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 task_id INTEGER REFERENCES simulation_tasks(id) ON DELETE SET NULL,
 role TEXT NOT NULL,
 kind TEXT NOT NULL,
 content TEXT NOT NULL DEFAULT '',
 payload_json TEXT,
 created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_messages_session_time ON messages(session_id,created_at_ms);
CREATE INDEX idx_messages_task ON messages(task_id);

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
 task_id INTEGER REFERENCES simulation_tasks(id) ON DELETE CASCADE,
 analysis_run_id INTEGER REFERENCES result_analysis_runs(id) ON DELETE CASCADE,
 artifact_id INTEGER NOT NULL REFERENCES artifacts(id),
 result_name TEXT NOT NULL,
 schema_json TEXT NOT NULL,
 schema_hash TEXT NOT NULL,
 row_count INTEGER NOT NULL CHECK(row_count>=0),
 size_bytes INTEGER NOT NULL CHECK(size_bytes>=0),
 created_at_ms INTEGER NOT NULL,
 CHECK((task_id IS NOT NULL)<>(analysis_run_id IS NOT NULL)),
 UNIQUE(task_id,result_name),
 UNIQUE(analysis_run_id,result_name)
);
CREATE INDEX idx_result_datasets_task ON result_datasets(task_id,id);
CREATE INDEX idx_result_datasets_analysis ON result_datasets(analysis_run_id,id);
CREATE TABLE result_analysis_inputs (
 analysis_run_id INTEGER NOT NULL REFERENCES result_analysis_runs(id) ON DELETE CASCADE,
 dataset_id INTEGER NOT NULL REFERENCES result_datasets(id) ON DELETE RESTRICT,
 alias TEXT NOT NULL,
 ordinal INTEGER NOT NULL,
 PRIMARY KEY(analysis_run_id,dataset_id),
 UNIQUE(analysis_run_id,alias),
 UNIQUE(analysis_run_id,ordinal)
);
CREATE TABLE dataset_views (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 dataset_id INTEGER NOT NULL REFERENCES result_datasets(id) ON DELETE CASCADE,
 name TEXT NOT NULL,
 kind TEXT NOT NULL CHECK(kind IN ('line','bar','scatter','heatmap','boxplot','histogram')),
 spec_json TEXT NOT NULL,
 is_default BOOLEAN NOT NULL DEFAULT 0,
 created_at_ms INTEGER NOT NULL,
 UNIQUE(dataset_id,name)
);
CREATE INDEX idx_dataset_views_dataset ON dataset_views(dataset_id,id);
CREATE TABLE task_comparisons (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 baseline_task_id INTEGER NOT NULL REFERENCES simulation_tasks(id) ON DELETE RESTRICT,
 candidate_task_id INTEGER NOT NULL REFERENCES simulation_tasks(id) ON DELETE RESTRICT,
 compatibility_json TEXT NOT NULL,
 analysis_run_id INTEGER NOT NULL UNIQUE REFERENCES result_analysis_runs(id) ON DELETE RESTRICT,
 created_at_ms INTEGER NOT NULL,
 CHECK(baseline_task_id<>candidate_task_id)
);
CREATE INDEX idx_comparisons_session_time ON task_comparisons(session_id,created_at_ms,id);

CREATE TABLE chat_search_documents (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 source_kind TEXT NOT NULL CHECK(source_kind IN ('session_title','message','attachment','task_summary')),
 source_id INTEGER,
 search_text TEXT NOT NULL,
 updated_at_ms INTEGER NOT NULL,
 UNIQUE(session_id,source_kind,source_id)
);
CREATE INDEX idx_chat_search_session_source ON chat_search_documents(session_id,source_kind,source_id);
CREATE VIRTUAL TABLE chat_search_fts USING fts5(search_text,content='chat_search_documents',content_rowid='id',tokenize='trigram');

CREATE TRIGGER validate_workspace_active_update BEFORE UPDATE OF active_version_id ON workspaces WHEN NEW.active_version_id IS NOT NULL BEGIN
 SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM model_versions WHERE id=NEW.active_version_id AND workspace_id=NEW.id AND status='ready') THEN RAISE(ABORT,'invalid active model version') END;
END;
CREATE TRIGGER validate_workspace_draft_update BEFORE UPDATE OF draft_version_id ON workspaces WHEN NEW.draft_version_id IS NOT NULL BEGIN
 SELECT CASE WHEN NOT EXISTS(SELECT 1 FROM model_versions WHERE id=NEW.draft_version_id AND workspace_id=NEW.id) THEN RAISE(ABORT,'invalid draft model version') END;
END;
CREATE TRIGGER immutable_model_versions BEFORE UPDATE OF workspace_id,status,source_kind,origin_kind,manifest_json,patch_json,content_hash,created_at_ms ON model_versions BEGIN SELECT RAISE(ABORT,'model versions are immutable'); END;
CREATE TRIGGER immutable_version_artifacts BEFORE UPDATE ON version_artifacts BEGIN SELECT RAISE(ABORT,'version artifacts are immutable'); END;
CREATE TRIGGER immutable_dataframe_bindings BEFORE UPDATE ON dataframe_bindings BEGIN SELECT RAISE(ABORT,'dataframe bindings are immutable'); END;

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
CREATE TRIGGER chat_attachment_ai AFTER INSERT ON ingestion_artifacts BEGIN
 INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) SELECT w.session_id,'attachment',new.id,new.file_name,new.created_at_ms FROM ingestions i JOIN workspaces w ON w.id=i.workspace_id WHERE i.id=new.ingestion_id;
END;
CREATE TRIGGER chat_attachment_ad AFTER DELETE ON ingestion_artifacts BEGIN DELETE FROM chat_search_documents WHERE source_kind='attachment' AND source_id=old.id; END;
CREATE TRIGGER chat_task_ai AFTER INSERT ON simulation_tasks BEGIN INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) VALUES(new.session_id,'task_summary',new.id,new.study_type||' '||new.status,new.created_at_ms); END;
CREATE TRIGGER chat_task_au AFTER UPDATE OF status,study_type ON simulation_tasks BEGIN UPDATE chat_search_documents SET search_text=new.study_type||' '||new.status,updated_at_ms=COALESCE(new.finished_at_ms,new.started_at_ms,new.created_at_ms) WHERE session_id=new.session_id AND source_kind='task_summary' AND source_id=new.id; END;
