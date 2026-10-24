# Backend Dependency Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Upgrade backend direct dependencies to current stable releases, remove dependencies that are provably unused, and preserve all V3 behavior.

**Architecture:** Treat `Cargo.toml` as the direct-dependency contract and `Cargo.lock` as its reproducible resolution. Audit usage across library, binaries, tests, and feature-gated code before deletion; limit Rust source changes to compatibility fixes required by dependency upgrades.

**Tech Stack:** Rust 2024, Cargo, Axum, Diesel, rig, Arrow/Parquet

**Spec:** `docs/superpowers/specs/2026-10-24-backend-dependency-refresh-design.md`

## Global Constraints

- Preserve the existing uncommitted package rename from `backend` to `zjudss`.
- Preserve REST/SSE contracts, database schema, Agent tools, adapter-only tensoreval execution, and Snappy Parquet results.
- Do not remove a dependency used by any library, binary, test, or enabled feature target.
- Do not perform unrelated refactoring or intentionally raise the MSRV.

## Review Focus

- Feature-gated `e2e-test` and `release-test` targets must still resolve all dependencies and compile.
- Test-only crates must remain in `dev-dependencies`, not be mistaken for unused crates.
- Major upgrades must not change authentication cookies, multipart limits, SSE recovery, or database behavior.
- Arrow and Parquet versions must stay aligned and preserve Snappy read/write behavior.
- The resolved graph must not retain avoidable duplicate major versions introduced by partial upgrades.

---

### Task 1: Establish the dependency baseline and usage map

**Files:**
- Inspect: `backend/Cargo.toml`
- Inspect: `backend/Cargo.lock`
- Inspect: `backend/src/**/*.rs`
- Inspect: `backend/tests/**/*.rs`

**Interfaces:**
- Consumes: the current manifest, lockfile, package rename, and all Cargo targets.
- Produces: a list of outdated direct dependencies, actual use sites, feature ownership, and deletion candidates.

- [ ] **Step 1: Record current resolution and available direct upgrades**

Run Cargo metadata/tree and registry-aware update checks without modifying source files. Compare direct dependencies as a coordinated group, especially Arrow/Parquet, Diesel/migrations, and rig/reqwest.

- [ ] **Step 2: Check direct dependency use across every target**

Search production, binary, test, and feature-gated sources, then run an all-target/all-feature compile with the unused-crate lint as corroborating evidence. Treat lint results per target to avoid false positives.

- [ ] **Step 3: Run the pre-upgrade test baseline**

Run: `cargo test --features e2e-test`

Expected: PASS, or a clearly documented pre-existing/environmental failure before manifest changes.

### Task 2: Upgrade and prune the dependency graph

**Files:**
- Modify: `backend/Cargo.toml`
- Modify: `backend/Cargo.lock`
- Modify if required: exact `backend/src/**/*.rs` or `backend/tests/**/*.rs` call sites rejected by upgraded APIs

**Interfaces:**
- Consumes: Task 1's upgrade and usage map.
- Produces: a consistent current direct-dependency set and any minimal API compatibility changes.

- [ ] **Step 1: Update direct dependency requirements**

Upgrade direct crates to current stable releases supported by the current toolchain. Keep related crates on compatible versions and preserve deliberately restricted features such as ZIP `default-features = false`.

- [ ] **Step 2: Regenerate and inspect the lockfile**

Resolve the graph, inspect version duplication and feature changes, and confirm that the package remains named `zjudss`.

- [ ] **Step 3: Remove proven-unused direct dependencies**

Delete only candidates with no required target use. Re-run `cargo check --all-targets --all-features` after each logical removal and restore any dependency whose absence breaks an intended target.

- [ ] **Step 4: Apply minimal compatibility fixes**

Change only call sites made invalid by upgraded public APIs. Do not change application behavior or public contracts.

### Task 3: Verify the refreshed backend

**Files:**
- Verify: all files changed by Tasks 1–2

**Interfaces:**
- Consumes: the upgraded manifest, lockfile, and compatibility changes.
- Produces: evidence that formatting, compilation, tests, and dependency constraints pass.

- [ ] **Step 1: Check formatting**

Run: `cargo fmt --all -- --check`

Expected: exit 0.

- [ ] **Step 2: Compile every target and feature combination**

Run: `cargo check --all-targets --all-features`

Expected: exit 0 with no missing or incompatible dependencies.

- [ ] **Step 3: Run the required backend suite**

Run: `cargo test --features e2e-test`

Expected: exit 0 and all tests pass.

- [ ] **Step 4: Review the final diff**

Confirm that changes are limited to dependency declarations, lock resolution, necessary compatibility fixes, and these design/plan records; report any environment-only test exclusions explicitly.
