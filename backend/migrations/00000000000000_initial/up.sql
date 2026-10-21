-- 初始 schema：会话、消息、仿真任务、任务文件、结果摘要、曲线数据
-- 约定：
--   时间戳一律 INTEGER（Unix 纪元，毫秒），列名后缀 _ms
--   布尔用 INTEGER 0/1
--   JSON 一律 TEXT（SQLite 无 JSONB），见 backend/docs/db-design.md §2.6
--   外键由运行期 PRAGMA foreign_keys = ON 启用（SQLite 默认关闭），见 §3.2
-- 建表顺序按依赖（父先于子）。

CREATE TABLE sessions (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    title         TEXT    NOT NULL DEFAULT '',
    created_at_ms INTEGER NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_sessions_updated ON sessions (updated_at_ms);

CREATE TABLE simulation_tasks (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id     INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    status         TEXT    NOT NULL,  -- pending | running | succeeded | failed | cancelled
    stage          TEXT,              -- model_check | script_gen | running | collecting（后端编排，非引擎概念）
    study_type     TEXT,              -- power_flow | qsts | harmonic | fault | ...
    params_json    TEXT    NOT NULL,  -- 参数快照（模型路径、study_type、result 选择、QSTS 时间设置、外部数据引用）
    exit_code      INTEGER,
    error_json     TEXT,              -- 结构化错误/诊断
    created_at_ms  INTEGER NOT NULL,
    started_at_ms  INTEGER,
    finished_at_ms INTEGER
);
CREATE INDEX idx_tasks_session_time ON simulation_tasks (session_id, created_at_ms);
CREATE INDEX idx_tasks_status       ON simulation_tasks (status);

CREATE TABLE messages (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    session_id    INTEGER NOT NULL REFERENCES sessions (id) ON DELETE CASCADE,
    task_id       INTEGER          REFERENCES simulation_tasks (id) ON DELETE SET NULL,
    role          TEXT    NOT NULL,  -- user | assistant | tool
    kind          TEXT    NOT NULL,  -- text | tool_call | tool_result | result_card | error
    content       TEXT    NOT NULL DEFAULT '',  -- 纯文本（卡片消息存一行可读摘要）
    payload_json  TEXT,               -- 卡片结构化数据（工具参数/结果摘要）
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_messages_session_time ON messages (session_id, created_at_ms);
CREATE INDEX idx_messages_task         ON messages (task_id);

CREATE TABLE task_files (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id       INTEGER NOT NULL REFERENCES simulation_tasks (id) ON DELETE CASCADE,
    role          TEXT    NOT NULL,  -- input_model | input_data | script | output_curve | report | diagnostics
    path          TEXT    NOT NULL,  -- 相对 DATA_DIR 的路径
    filename      TEXT    NOT NULL,
    format        TEXT,              -- dss | csv | parquet | rss | json
    result_name   TEXT,              -- output_curve 对应的结果名（bus_phase/branch_flow/...）
    size_bytes    INTEGER,
    sha256        TEXT,
    created_at_ms INTEGER NOT NULL
);
CREATE INDEX idx_files_task_role ON task_files (task_id, role);
CREATE INDEX idx_files_sha256    ON task_files (sha256);

CREATE TABLE task_results (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id             INTEGER NOT NULL REFERENCES simulation_tasks (id) ON DELETE CASCADE,
    result_name         TEXT    NOT NULL,  -- summary | violations | bus_phase | branch_flow | ...
    row_count           INTEGER NOT NULL DEFAULT 0,
    -- 归一化关键指标（仅 summary 结果填充，其余为 NULL），用于排序/对比
    converged           INTEGER,  -- 0/1
    violation_count     INTEGER,
    p_loss_w            REAL,
    q_loss_var          REAL,
    max_violation_ratio REAL,
    metrics_json        TEXT    NOT NULL,  -- 完整摘要/小结果全量（summary/violations）
    created_at_ms       INTEGER NOT NULL
);
CREATE UNIQUE INDEX idx_results_task_name ON task_results (task_id, result_name);
CREATE INDEX idx_results_task           ON task_results (task_id);

-- 可选：需在 SQL 内按 series_key + time_s 查询/聚合的时序点。
-- 全量曲线推荐存文件（见 db-design.md §5.2），v1 可不启用本表。
CREATE TABLE curve_data (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    task_id     INTEGER NOT NULL REFERENCES simulation_tasks (id) ON DELETE CASCADE,
    result_name TEXT    NOT NULL,
    series_key  TEXT    NOT NULL,  -- 设备/母线/相标识（如 "bus.phase"）
    time_s      REAL    NOT NULL,  -- 仿真时间（秒）
    variable    TEXT    NOT NULL,  -- v_mag | p | q | i_mag | ...
    value       REAL    NOT NULL
);
CREATE INDEX idx_curve_task_name_key_time ON curve_data (task_id, result_name, series_key, time_s);
