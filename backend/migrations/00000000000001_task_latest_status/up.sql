-- 会话最新任务状态推导的覆盖索引：
-- 单会话用 WHERE session_id = ? ORDER BY id DESC LIMIT 1，
-- 批量用 MAX(id) GROUP BY session_id，(session_id, id) 覆盖两者。
CREATE INDEX idx_tasks_session_id ON simulation_tasks (session_id, id);
