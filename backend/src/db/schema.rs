//! Diesel 表结构（`table!` 宏），与 `migrations/00000000000000_initial/up.sql` 一一对应。
//!
//! 手写以匹配迁移 SQL（若用 `diesel print-schema` 会生成等价内容）。类型约定：
//! - `Integer` = i32，`BigInt` = i64（毫秒时间戳 / 文件字节数），`Double` = f64
//! - `Bool` = bool（SQLite 存 0/1），`Nullable<T>` = `Option<T>`

diesel::table! {
    sessions (id) {
        id -> Integer,
        title -> Text,
        created_at_ms -> BigInt,
        updated_at_ms -> BigInt,
        access_scope_kind -> Text,
        access_scope_key -> Text,
        storage_scope_id -> Nullable<Integer>,
    }
}

diesel::table! {
    storage_scopes (id) {
        id -> Integer,
        kind -> Text,
        scope_key -> Text,
        created_at_ms -> BigInt,
        artifact_limit -> BigInt,
        byte_limit -> BigInt,
        warning_ratio -> Double,
        used_artifacts -> BigInt,
        used_bytes -> BigInt,
        updated_at_ms -> BigInt,
    }
}

diesel::table! {
    artifacts (id) {
        id -> Integer,
        sha256 -> Text,
        size_bytes -> BigInt,
        storage_path -> Text,
        format -> Text,
        media_type -> Nullable<Text>,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    storage_scope_artifacts (storage_scope_id, artifact_id) {
        storage_scope_id -> Integer,
        artifact_id -> Integer,
        ref_count -> BigInt,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    artifact_gc_failures (relative_path) {
        relative_path -> Text,
        error -> Text,
        attempts -> BigInt,
        updated_at_ms -> BigInt,
    }
}

diesel::table! {
    simulation_tasks (id) {
        id -> Integer,
        session_id -> Integer,
        status -> Text,
        stage -> Nullable<Text>,
        study_type -> Nullable<Text>,
        params_json -> Text,
        exit_code -> Nullable<Integer>,
        error_json -> Nullable<Text>,
        created_at_ms -> BigInt,
        started_at_ms -> Nullable<BigInt>,
        finished_at_ms -> Nullable<BigInt>,
    }
}

diesel::table! {
    messages (id) {
        id -> Integer,
        session_id -> Integer,
        task_id -> Nullable<Integer>,
        role -> Text,
        kind -> Text,
        content -> Text,
        payload_json -> Nullable<Text>,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    task_files (id) {
        id -> Integer,
        task_id -> Integer,
        role -> Text,
        path -> Text,
        filename -> Text,
        format -> Nullable<Text>,
        result_name -> Nullable<Text>,
        size_bytes -> Nullable<BigInt>,
        sha256 -> Nullable<Text>,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    task_results (id) {
        id -> Integer,
        task_id -> Integer,
        result_name -> Text,
        row_count -> Integer,
        converged -> Nullable<Bool>,
        violation_count -> Nullable<Integer>,
        p_loss_w -> Nullable<Double>,
        q_loss_var -> Nullable<Double>,
        max_violation_ratio -> Nullable<Double>,
        metrics_json -> Text,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    curve_data (id) {
        id -> Integer,
        task_id -> Integer,
        result_name -> Text,
        series_key -> Text,
        time_s -> Double,
        variable -> Text,
        value -> Double,
    }
}

diesel::joinable!(sessions -> storage_scopes (storage_scope_id));
diesel::joinable!(storage_scope_artifacts -> artifacts (artifact_id));
diesel::joinable!(storage_scope_artifacts -> storage_scopes (storage_scope_id));

diesel::allow_tables_to_appear_in_same_query!(
    sessions,
    storage_scopes,
    artifacts,
    storage_scope_artifacts,
    artifact_gc_failures,
    simulation_tasks,
    messages,
    task_files,
    task_results,
    curve_data,
);
