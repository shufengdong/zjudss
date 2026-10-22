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
    workspaces (id) {
        id -> Integer,
        session_id -> Integer,
        active_version_id -> Nullable<Integer>,
        draft_version_id -> Nullable<Integer>,
        created_at_ms -> BigInt,
        updated_at_ms -> BigInt,
    }
}

diesel::table! {
    model_versions (id) {
        id -> Integer,
        workspace_id -> Integer,
        parent_version_id -> Nullable<Integer>,
        status -> Text,
        source_kind -> Text,
        origin_kind -> Text,
        manifest_json -> Text,
        patch_json -> Nullable<Text>,
        content_hash -> Text,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    version_artifacts (id) {
        id -> Integer,
        version_id -> Integer,
        artifact_id -> Integer,
        role -> Text,
        relative_path -> Nullable<Text>,
        display_name -> Text,
        source_artifact_id -> Nullable<Integer>,
        sheet_name -> Nullable<Text>,
    }
}

diesel::table! {
    dataframe_bindings (id) {
        id -> Integer,
        version_artifact_id -> Integer,
        table_kind -> Nullable<Text>,
        confidence -> Nullable<Double>,
        resolution -> Text,
        columns_json -> Text,
        evidence_json -> Text,
    }
}

diesel::table! {
    ingestions (id) {
        id -> Integer,
        workspace_id -> Integer,
        client_batch_id -> Text,
        origin_kind -> Text,
        source_ref_json -> Nullable<Text>,
        base_version_id -> Nullable<Integer>,
        published_version_id -> Nullable<Integer>,
        status -> Text,
        stages_json -> Text,
        error_code -> Nullable<Text>,
        error_message -> Nullable<Text>,
        created_at_ms -> BigInt,
        updated_at_ms -> BigInt,
    }
}

diesel::table! {
    ingestion_artifacts (id) {
        id -> Integer,
        ingestion_id -> Integer,
        client_file_id -> Text,
        artifact_id -> Integer,
        file_name -> Text,
        relative_path -> Nullable<Text>,
        media_type -> Nullable<Text>,
        role -> Text,
        source_artifact_id -> Nullable<Integer>,
        sheet_name -> Nullable<Text>,
        status -> Text,
        error_code -> Nullable<Text>,
        error_message -> Nullable<Text>,
        created_at_ms -> BigInt,
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
        model_version_id -> Nullable<Integer>,
        study_spec_json -> Nullable<Text>,
        script_artifact_id -> Nullable<Integer>,
        agent_bundle_version -> Nullable<Text>,
        agent_skill_refs_json -> Nullable<Text>,
        engine_contract_version -> Nullable<Text>,
    }
}

diesel::table! {
    result_analysis_runs (id) {
        id -> Integer,
        session_id -> Integer,
        request_message_id -> Nullable<Integer>,
        intent_json -> Text,
        script_artifact_id -> Integer,
        status -> Text,
        stage -> Text,
        error_json -> Nullable<Text>,
        agent_bundle_version -> Text,
        agent_skill_refs_json -> Text,
        engine_contract_version -> Text,
        created_at_ms -> BigInt,
        started_at_ms -> Nullable<BigInt>,
        finished_at_ms -> Nullable<BigInt>,
    }
}

diesel::table! {
    result_analysis_inputs (analysis_run_id, dataset_id) {
        analysis_run_id -> Integer,
        dataset_id -> Integer,
        alias -> Text,
        ordinal -> Integer,
    }
}

diesel::table! {
    result_datasets (id) {
        id -> Integer,
        task_id -> Nullable<Integer>,
        analysis_run_id -> Nullable<Integer>,
        artifact_id -> Integer,
        result_name -> Text,
        schema_json -> Text,
        schema_hash -> Text,
        row_count -> BigInt,
        size_bytes -> BigInt,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    dataset_views (id) {
        id -> Integer,
        dataset_id -> Integer,
        name -> Text,
        kind -> Text,
        spec_json -> Text,
        is_default -> Bool,
        created_at_ms -> BigInt,
    }
}
diesel::table! {
    task_comparisons (id) {
        id -> Integer,
        session_id -> Integer,
        baseline_task_id -> Integer,
        candidate_task_id -> Integer,
        compatibility_json -> Text,
        analysis_run_id -> Integer,
        created_at_ms -> BigInt,
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
diesel::joinable!(workspaces -> sessions (session_id));
diesel::joinable!(model_versions -> workspaces (workspace_id));
diesel::joinable!(version_artifacts -> model_versions (version_id));
diesel::joinable!(dataframe_bindings -> version_artifacts (version_artifact_id));
diesel::joinable!(ingestions -> workspaces (workspace_id));
diesel::joinable!(ingestion_artifacts -> ingestions (ingestion_id));
diesel::joinable!(storage_scope_artifacts -> artifacts (artifact_id));
diesel::joinable!(storage_scope_artifacts -> storage_scopes (storage_scope_id));
diesel::joinable!(result_analysis_runs -> sessions (session_id));
diesel::joinable!(result_analysis_inputs -> result_analysis_runs (analysis_run_id));
diesel::joinable!(result_datasets -> artifacts (artifact_id));
diesel::joinable!(dataset_views -> result_datasets (dataset_id));
diesel::joinable!(task_comparisons -> sessions (session_id));

diesel::allow_tables_to_appear_in_same_query!(
    sessions,
    storage_scopes,
    artifacts,
    storage_scope_artifacts,
    artifact_gc_failures,
    workspaces,
    model_versions,
    version_artifacts,
    dataframe_bindings,
    ingestions,
    ingestion_artifacts,
    simulation_tasks,
    messages,
    task_files,
    task_results,
    curve_data,
    result_analysis_runs,
    result_analysis_inputs,
    result_datasets,
    dataset_views,
    task_comparisons,
);
