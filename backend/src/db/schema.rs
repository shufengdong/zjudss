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
        user_id -> Text,
        title_source -> Text,
    }
}

diesel::table! {
    users (id) {
        id -> Text,
        name -> Text,
        password_hash -> Text,
        storage_scope_id -> Integer,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    auth_sessions (token_hash) {
        token_hash -> Text,
        user_id -> Text,
        created_at_ms -> BigInt,
        last_seen_at_ms -> BigInt,
        expires_at_ms -> BigInt,
    }
}

diesel::table! {
    system_metadata (key) {
        key -> Text,
        value -> Text,
    }
}
diesel::table! {
    chat_search_documents (id) {
        id -> Integer,
        session_id -> Integer,
        source_kind -> Text,
        source_id -> Nullable<Integer>,
        search_text -> Text,
        updated_at_ms -> BigInt,
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
        created_at_ms -> BigInt,
        updated_at_ms -> BigInt,
    }
}

diesel::table! {
    workspace_drafts (id) {
        id -> Integer,
        workspace_id -> Integer,
        base_version_id -> Nullable<Integer>,
        revision -> BigInt,
        diagnostics_json -> Text,
        created_at_ms -> BigInt,
        updated_at_ms -> BigInt,
    }
}

diesel::table! {
    draft_dataframes (id) {
        id -> Integer,
        draft_id -> Integer,
        table_name -> Text,
        artifact_id -> Integer,
        schema_json -> Text,
        row_count -> BigInt,
        content_hash -> Text,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    model_dataframes (id) {
        id -> Integer,
        version_id -> Integer,
        table_name -> Text,
        artifact_id -> Integer,
        schema_json -> Text,
        row_count -> BigInt,
        content_hash -> Text,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    model_imports (id) {
        id -> Integer,
        workspace_id -> Integer,
        session_id -> Integer,
        client_import_id -> Text,
        source_kind -> Text,
        source_summary_json -> Text,
        source_sha256 -> Text,
        base_draft_id -> Nullable<Integer>,
        base_draft_revision -> Nullable<BigInt>,
        status -> Text,
        stages_json -> Text,
        diagnostics_json -> Text,
        error_code -> Nullable<Text>,
        error_message -> Nullable<Text>,
        created_at_ms -> BigInt,
        updated_at_ms -> BigInt,
    }
}

diesel::table! {
    model_import_candidates (id) {
        id -> Integer,
        import_id -> Integer,
        candidate_id -> Text,
        candidate_kind -> Text,
        display_name -> Text,
        table_name -> Nullable<Text>,
        artifact_id -> Nullable<Integer>,
        schema_json -> Nullable<Text>,
        row_count -> Nullable<BigInt>,
        content_hash -> Nullable<Text>,
        evidence_json -> Text,
        selected -> Bool,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    model_import_selections (import_id, candidate_id) {
        import_id -> Integer,
        candidate_id -> Text,
        table_name -> Nullable<Text>,
        selected -> Bool,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    publish_receipts (id) {
        id -> Integer,
        draft_id -> Integer,
        draft_revision -> BigInt,
        client_publish_id -> Text,
        version_id -> Integer,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    script_resources (id) {
        id -> Integer,
        session_id -> Integer,
        artifact_id -> Integer,
        name -> Text,
        source_kind -> Text,
        content_hash -> Text,
        validation_status -> Text,
        required_aliases_json -> Text,
        outputs_json -> Text,
        diagnostics_json -> Text,
        agent_bundle_version -> Nullable<Text>,
        agent_skill_refs_json -> Text,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    simulation_runs (id) {
        id -> Integer,
        session_id -> Integer,
        model_version_id -> Integer,
        model_content_hash -> Text,
        script_resource_id -> Integer,
        script_artifact_id -> Integer,
        script_content_hash -> Text,
        client_run_id -> Text,
        status -> Text,
        stage -> Text,
        engine_contract_version -> Text,
        agent_bundle_version -> Nullable<Text>,
        agent_skill_refs_json -> Text,
        exit_code -> Nullable<Integer>,
        error_json -> Nullable<Text>,
        created_at_ms -> BigInt,
        started_at_ms -> Nullable<BigInt>,
        finished_at_ms -> Nullable<BigInt>,
        deleted_at_ms -> Nullable<BigInt>,
        tombstone_json -> Nullable<Text>,
    }
}

diesel::table! {
    dataset_lineage_snapshots (id) {
        id -> Integer,
        derived_dataset_id -> Integer,
        source_dataset_id -> Integer,
        source_name -> Text,
        source_schema_json -> Text,
        source_schema_hash -> Text,
        source_producer_json -> Text,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    version_deletion_previews (token_hash) {
        token_hash -> Text,
        user_id -> Text,
        version_id -> Integer,
        impact_hash -> Text,
        impact_json -> Text,
        expires_at_ms -> BigInt,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    model_versions (id) {
        id -> Integer,
        workspace_id -> Integer,
        content_hash -> Text,
        source_summary_json -> Text,
        engine_contract_version -> Text,
        created_at_ms -> BigInt,
        deleted_at_ms -> Nullable<BigInt>,
        tombstone_json -> Nullable<Text>,
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
        source_name -> Text,
        source_schema_json -> Text,
        source_schema_hash -> Text,
        source_producer_json -> Text,
    }
}

diesel::table! {
    result_datasets (id) {
        id -> Integer,
        run_id -> Nullable<Integer>,
        analysis_run_id -> Nullable<Integer>,
        artifact_id -> Nullable<Integer>,
        result_name -> Text,
        schema_json -> Text,
        schema_hash -> Text,
        row_count -> BigInt,
        size_bytes -> BigInt,
        created_at_ms -> BigInt,
        deleted_at_ms -> Nullable<BigInt>,
        tombstone_json -> Nullable<Text>,
    }
}

diesel::table! {
    dataset_views (id) {
        id -> Integer,
        source_dataset_id -> Integer,
        view_dataset_id -> Integer,
        name -> Text,
        kind -> Text,
        spec_json -> Text,
        is_default -> Bool,
        created_at_ms -> BigInt,
    }
}
diesel::table! {
    run_comparisons (id) {
        id -> Integer,
        session_id -> Integer,
        baseline_run_id -> Integer,
        candidate_run_id -> Integer,
        baseline_snapshot_json -> Text,
        candidate_snapshot_json -> Text,
        compatibility_json -> Text,
        analysis_run_id -> Integer,
        created_at_ms -> BigInt,
    }
}

diesel::table! {
    messages (id) {
        id -> Integer,
        session_id -> Integer,
        run_id -> Nullable<Integer>,
        role -> Text,
        kind -> Text,
        content -> Text,
        payload_json -> Nullable<Text>,
        created_at_ms -> BigInt,
    }
}

diesel::joinable!(sessions -> users (user_id));
diesel::joinable!(users -> storage_scopes (storage_scope_id));
diesel::joinable!(auth_sessions -> users (user_id));
diesel::joinable!(workspaces -> sessions (session_id));
diesel::joinable!(model_versions -> workspaces (workspace_id));
diesel::joinable!(storage_scope_artifacts -> artifacts (artifact_id));
diesel::joinable!(storage_scope_artifacts -> storage_scopes (storage_scope_id));
diesel::joinable!(result_analysis_runs -> sessions (session_id));
diesel::joinable!(result_analysis_inputs -> result_analysis_runs (analysis_run_id));
diesel::joinable!(run_comparisons -> sessions (session_id));
diesel::joinable!(chat_search_documents -> sessions (session_id));

diesel::allow_tables_to_appear_in_same_query!(
    sessions,
    users,
    auth_sessions,
    system_metadata,
    storage_scopes,
    artifacts,
    storage_scope_artifacts,
    artifact_gc_failures,
    workspaces,
    workspace_drafts,
    draft_dataframes,
    model_dataframes,
    model_imports,
    model_import_candidates,
    model_import_selections,
    publish_receipts,
    script_resources,
    simulation_runs,
    dataset_lineage_snapshots,
    version_deletion_previews,
    model_versions,
    messages,
    result_analysis_runs,
    result_analysis_inputs,
    result_datasets,
    dataset_views,
    run_comparisons,
    chat_search_documents,
);
