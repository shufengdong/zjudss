-- 回滚初始 schema：按依赖逆序删除（子先于父）。
DROP TABLE IF EXISTS curve_data;
DROP TABLE IF EXISTS task_results;
DROP TABLE IF EXISTS task_files;
DROP TABLE IF EXISTS messages;
DROP TABLE IF EXISTS simulation_tasks;
DROP TABLE IF EXISTS sessions;
