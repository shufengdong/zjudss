ALTER TABLE sessions ADD COLUMN title_source TEXT NOT NULL DEFAULT 'default' CHECK(title_source IN ('default','auto','user'));

CREATE TABLE chat_search_documents (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 session_id INTEGER NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
 source_kind TEXT NOT NULL CHECK(source_kind IN ('session_title','message','attachment','task_summary')),
 source_id INTEGER,
 search_text TEXT NOT NULL,
 updated_at_ms BIGINT NOT NULL,
 UNIQUE(session_id,source_kind,source_id)
);
CREATE INDEX idx_chat_search_session_source ON chat_search_documents(session_id,source_kind,source_id);
CREATE VIRTUAL TABLE chat_search_fts USING fts5(search_text, content='chat_search_documents', content_rowid='id', tokenize='trigram');

CREATE TRIGGER chat_docs_ai AFTER INSERT ON chat_search_documents BEGIN
 INSERT INTO chat_search_fts(rowid,search_text) VALUES(new.id,new.search_text);
END;
CREATE TRIGGER chat_docs_ad AFTER DELETE ON chat_search_documents BEGIN
 INSERT INTO chat_search_fts(chat_search_fts,rowid,search_text) VALUES('delete',old.id,old.search_text);
END;
CREATE TRIGGER chat_docs_au AFTER UPDATE ON chat_search_documents BEGIN
 INSERT INTO chat_search_fts(chat_search_fts,rowid,search_text) VALUES('delete',old.id,old.search_text);
 INSERT INTO chat_search_fts(rowid,search_text) VALUES(new.id,new.search_text);
END;
CREATE TRIGGER chat_session_ai AFTER INSERT ON sessions BEGIN
 INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) VALUES(new.id,'session_title',new.id,new.title,new.updated_at_ms);
END;
CREATE TRIGGER chat_session_au AFTER UPDATE OF title ON sessions BEGIN
 UPDATE chat_search_documents SET search_text=new.title,updated_at_ms=new.updated_at_ms WHERE session_id=new.id AND source_kind='session_title' AND source_id=new.id;
END;
CREATE TRIGGER chat_message_ai AFTER INSERT ON messages WHEN new.role IN ('user','assistant') BEGIN
 INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) VALUES(new.session_id,'message',new.id,new.content,new.created_at_ms);
END;
CREATE TRIGGER chat_message_ad AFTER DELETE ON messages BEGIN
 DELETE FROM chat_search_documents WHERE session_id=old.session_id AND source_kind='message' AND source_id=old.id;
END;
CREATE TRIGGER chat_attachment_ai AFTER INSERT ON ingestion_artifacts BEGIN
 INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms)
 SELECT w.session_id,'attachment',new.id,new.file_name,new.created_at_ms FROM ingestions i JOIN workspaces w ON w.id=i.workspace_id WHERE i.id=new.ingestion_id;
END;
CREATE TRIGGER chat_attachment_ad AFTER DELETE ON ingestion_artifacts BEGIN
 DELETE FROM chat_search_documents WHERE source_kind='attachment' AND source_id=old.id;
END;
CREATE TRIGGER chat_task_ai AFTER INSERT ON simulation_tasks BEGIN
 INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) VALUES(new.session_id,'task_summary',new.id,COALESCE(new.study_type,'')||' '||new.status,new.created_at_ms);
END;
CREATE TRIGGER chat_task_au AFTER UPDATE OF status,study_type ON simulation_tasks BEGIN
 UPDATE chat_search_documents SET search_text=COALESCE(new.study_type,'')||' '||new.status,updated_at_ms=COALESCE(new.finished_at_ms,new.started_at_ms,new.created_at_ms) WHERE session_id=new.session_id AND source_kind='task_summary' AND source_id=new.id;
END;

INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) SELECT id,'session_title',id,title,updated_at_ms FROM sessions;
INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) SELECT session_id,'message',id,content,created_at_ms FROM messages WHERE role IN ('user','assistant');
INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) SELECT session_id,'task_summary',id,COALESCE(study_type,'')||' '||status,COALESCE(finished_at_ms,started_at_ms,created_at_ms) FROM simulation_tasks;
INSERT INTO chat_search_documents(session_id,source_kind,source_id,search_text,updated_at_ms) SELECT w.session_id,'attachment',ia.id,ia.file_name,ia.created_at_ms FROM ingestion_artifacts ia JOIN ingestions i ON i.id=ia.ingestion_id JOIN workspaces w ON w.id=i.workspace_id;
