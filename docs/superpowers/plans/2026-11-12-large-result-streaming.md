# Large Result Streaming Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Set the result file limit to 2 GiB, keep an 8 GiB per-run total budget, and stream protected downloads without buffering full result files in backend or browser memory.

**Architecture:** Runtime and adapter defaults share the exact binary byte limits. The existing authorized file handle is converted into an Axum body backed by bounded asynchronous reads and an exact `Content-Length`; the result workbench exposes the same authenticated GET as a direct download link. No result partitioning or persistence schema change is introduced.

**Tech Stack:** Rust, Axum, Tokio, React, TypeScript, Vitest.

**Spec:** `docs/superpowers/specs/2026-11-12-large-result-streaming-design.md`

## Global Constraints

- All tensoreval calls remain behind `backend/src/adapters/`.
- Results remain complete immutable Snappy Parquet artifacts; no truncation, sampling, CSV fallback, or partial publication.
- Preserve current authenticated download routes, authorization, filenames, and media types.
- Modify `master` directly as required by `AGENTS.md`; do not touch the user's existing agent manifest changes.

## Review Focus

- A file exactly 2 GiB remains allowed and a file one byte larger is rejected by the existing `>` limit semantics; covered by existing adapter boundary tests plus the configured literal.
- A download read failure after headers becomes a body stream error rather than an unbounded allocation; covered by bounded-chunk response construction.
- Empty files return `Content-Length: 0` and an empty body; covered by the attachment unit test.
- Result downloads preserve cookie-authenticated same-origin routing through `apiUrl`; covered by the workbench component test.
- Total output remains able to contain multiple legal files up to 8 GiB; covered by the runtime default test.

---

### Task 1: Result capacity defaults and documentation

**Files:**
- Modify: `backend/tests/app_bootstrap.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/src/adapters/process/output.rs`
- Modify: `backend/.env.example`
- Modify: `docs/v4-deployment.md`
- Modify: `docs/development-status.md`

**Interfaces:**
- Produces runtime and adapter defaults of `max_file_bytes = 2147483648` and `max_total_bytes = 8589934592`.

- [ ] Write the failing runtime-default assertions for 2 GiB per file and 8 GiB total.
- [ ] Run `cargo test --features e2e-test --test app_bootstrap runtime_config_uses_v4_defaults_and_rejects_zero_or_inconsistent_limits` and confirm the old 512 MiB/2 GiB defaults fail.
- [ ] Update both default constructors, the environment template, and current deployment/development documentation.
- [ ] Re-run the focused test and confirm it passes.

### Task 2: Bounded backend streaming and direct result download

**Files:**
- Modify: `backend/src/api/attachment.rs`
- Modify: `frontend/src/api/client.ts`
- Modify: `frontend/src/api/client.test.ts`
- Modify: `frontend/src/components/datasets/DatasetWorkbench.tsx`
- Create: `frontend/src/components/datasets/DatasetWorkbench.test.tsx`
- Modify: `docs/api-design.md`
- Modify: `docs/frontend-design.md`

**Interfaces:**
- Produces `attachment::response` with exact `Content-Length` and bounded body chunks.
- Produces `datasetDownloadUrl(id: string): string` for a same-origin authenticated direct link.

- [ ] Add backend tests for bounded multi-chunk and empty-file responses; run the focused Rust tests and observe failure against the buffered body.
- [ ] Implement bounded asynchronous file streaming and exact length headers; rerun the focused Rust tests to green.
- [ ] Add the frontend workbench test for a direct dataset download link and update the API client tests so dataset download no longer uses Fetch/Blob; run the focused Vitest files and observe failure.
- [ ] Implement `datasetDownloadUrl`, replace the workbench button with a direct link, and update API/frontend docs; rerun focused Vitest files to green.

### Task 3: Full verification and runtime restart

**Files:**
- Modify: `backend/.env` (ignored runtime configuration only)

**Interfaces:**
- Consumes the two limits and streaming behavior from Tasks 1–2.

- [ ] Set the local runtime to `RESULT_MAX_FILE_BYTES=2147483648` and `RESULT_MAX_TOTAL_BYTES=8589934592`.
- [ ] Run backend format, Clippy, and the full e2e-feature test suite.
- [ ] Run the full frontend Vitest suite and production build.
- [ ] Restart backend and frontend, then verify the health endpoint and frontend HTTP response.
