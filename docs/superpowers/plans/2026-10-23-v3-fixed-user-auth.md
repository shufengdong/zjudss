# V3 Fixed User Authentication Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the unreleased V2 instance/trusted-proxy identity abstraction with CSV-seeded fixed users, opaque 30-day database sessions, strict per-user ownership, and a login/logout UI.

**Architecture:** The initial migration becomes the only V3 schema and owns `users`, `auth_sessions`, and user-bound business sessions. Startup seeds users transactionally from the embedded CSV before accepting traffic. A router middleware authenticates an HttpOnly cookie, injects a typed user identity, renews expiring sessions, and supplies the auth-session identity needed to terminate SSE on expiry/logout. Every repository continues to receive an explicit scope, but the scope is now a `UserId` backed by a database foreign key rather than caller-controlled strings.

**Tech Stack:** Rust 2024, Axum 0.8, Diesel/SQLite, `argon2`, `rand_core`, `cookie`, Tokio broadcast/SSE, React 19, TypeScript, Vitest, Playwright.

**Spec:** `docs/superpowers/specs/2026-10-23-v3-fixed-user-auth-design.md`

## Global Constraints

- V3 starts from an empty SQLite database and empty business `DATA_DIR`; do not add V2 migration, compatibility reads, or trusted-proxy fallback.
- Never persist or log clear-text passwords or raw session tokens. The embedded CSV is the only user provisioning source.
- All protected repositories must accept `&UserId` explicitly; a handler may not accept a user ID from the request body/query to establish ownership.
- Cross-user and missing domain resources remain indistinguishable `404 not_found`; missing/expired authentication is `401 authentication_required`.
- `GET /health`, login, and idempotent logout are the only public routes.
- Preserve the user's uncommitted `backend/resources/init_users.csv`; implementation may read and embed it, but must not rewrite it.
- Each task follows red-green-refactor and commits only its listed files plus directly necessary generated lockfile changes.

## Review Focus

- No remaining `instance`, `trusted_proxy`, `AccessScope`, `StorageScopeRef`, `AUTH_MODE`, or proxy-header production path.
- Seed mismatch fails before HTTP bind and leaves no partial users/scopes.
- Cookie lifetime and database lifetime derive from the same TTL, and SSE heartbeat never extends it.
- All SQL paths join through the authenticated user's business session; direct primary-key lookups cannot bypass isolation.
- React state is keyed/reset by user identity so a second login cannot flash the previous user's data.

---

### Task 1: Replace the initial identity schema and add transactional CSV seeding

**Files:**
- Modify: `backend/Cargo.toml`
- Modify: `backend/Cargo.lock`
- Modify: `backend/migrations/00000000000000_initial/up.sql`
- Modify: `backend/migrations/00000000000000_initial/down.sql`
- Modify: `backend/src/db/schema.rs`
- Modify: `backend/src/db/models.rs`
- Modify: `backend/src/db/mod.rs`
- Add: `backend/src/db/repositories/user.rs`
- Add: `backend/src/auth/seed.rs`
- Modify: `backend/src/auth/mod.rs`
- Add test: `backend/tests/user_seed.rs`
- Modify test support: `backend/tests/common/mod.rs`

- [x] Write failing seed and schema tests.

  Cover exactly: the current CSV yields its actual row count; stored hashes begin with `$argon2id$` and differ from IDs; headers must be exactly `ID,NAME`; empty/duplicate/control-character/extra-column input rolls back all inserts; identical bytes are idempotent; different bytes against a populated database return `user_seed_mismatch_reset_required`; `sessions.user_id` is required and `access_scope_*` columns no longer exist.

- [x] Run `cd backend && cargo test --test user_seed --features e2e-test` and confirm the new tests fail for missing schema/service.

- [x] Rewrite the empty-database initial migration.

  Add:

  ```sql
  users(id TEXT PRIMARY KEY, name TEXT NOT NULL, password_hash TEXT NOT NULL,
        storage_scope_id INTEGER NOT NULL UNIQUE REFERENCES storage_scopes(id),
        created_at_ms BIGINT NOT NULL);
  auth_sessions(token_hash TEXT PRIMARY KEY, user_id TEXT NOT NULL REFERENCES users(id),
        created_at_ms BIGINT NOT NULL, last_seen_at_ms BIGINT NOT NULL,
        expires_at_ms BIGINT NOT NULL);
  system_metadata(key TEXT PRIMARY KEY, value TEXT NOT NULL);
  ```

  Replace `sessions.access_scope_kind/access_scope_key/storage_scope_id` with required `sessions.user_id`; retain per-user storage scopes but constrain their V3 `kind='user'` and `scope_key=user.id`. Add indexes on auth-session user/expiry and business-session user/update time. Order `down.sql` by foreign-key dependency.

- [x] Add the seed API and embedded build input.

  ```rust
  pub const INIT_USERS_CSV: &[u8] = include_bytes!("../../resources/init_users.csv");

  pub fn seed_users(
      conn: &mut SqliteConnection,
      csv_bytes: &[u8],
      now_ms: i64,
      limits: StorageLimits,
  ) -> Result<UserSeedSummary, UserSeedError>;

  pub struct UserSeedSummary {
      pub user_count: usize,
      pub seed_sha256: String,
      pub initialized: bool,
  }
  ```

  Parse and validate the complete CSV before the insert transaction. Use Argon2id with documented constants and a distinct random salt per row. Insert the user storage scope and user atomically, then write `init_users_sha256`. Never include IDs in errors or logs.

- [x] Update Diesel table declarations/models and export `UserRepo`; give `backend/tests/common/mod.rs` a seed helper that accepts fixture CSV bytes rather than silently depending on a specific row count.

- [x] Run `cd backend && cargo test --test user_seed --features e2e-test` and confirm it passes.

- [x] Commit: `git add backend/Cargo.toml backend/Cargo.lock backend/migrations backend/src/db backend/src/auth backend/tests/user_seed.rs backend/tests/common/mod.rs && git commit -m "feat: seed fixed v3 users"` (explicitly exclude `backend/resources/init_users.csv`).

### Task 2: Implement opaque auth sessions, cookies, and protected routing

**Files:**
- Add: `backend/src/auth/session.rs`
- Replace: `backend/src/auth/mod.rs`
- Remove: `backend/src/auth/trusted_proxy.rs`
- Modify: `backend/src/api/auth.rs`
- Modify: `backend/src/api/mod.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/src/main.rs`
- Modify: `backend/src/bin/e2e-server.rs`
- Replace test: `backend/tests/auth_scope.rs`
- Modify test support: `backend/tests/common/mod.rs`
- Modify: `backend/.env.example`

- [x] Write failing HTTP behavior tests for login, `/api/me`, protected routes, renewal, expiry, and logout.

  Assert `invalid_credentials` is identical for unknown ID/wrong password; successful login returns `{user:{id,name}}` and `zjudss_session` with `HttpOnly; SameSite=Lax; Path=/; Max-Age=<TTL>`; raw token is absent from JSON/database; no/expired/unknown cookie returns `authentication_required`; renewal happens only after half-TTL and commits before setting a cookie; logout always returns 204 and expires the cookie; one browser logout does not revoke the user's other auth session.

- [x] Run `cd backend && cargo test --test auth_scope --features e2e-test` and confirm failure on the old proxy contract.

- [x] Implement these core types and services:

  ```rust
  #[derive(Clone, Debug, Eq, Hash, PartialEq)]
  pub struct UserId(pub String);

  #[derive(Clone, Debug)]
  pub struct CurrentUser {
      pub id: UserId,
      pub name: String,
      pub storage_scope_id: i32,
      pub auth_session_hash: String,
  }

  #[derive(Clone)]
  pub struct AuthService { /* repo, ttl, cookie settings, revocations */ }

  pub async fn require_user(
      State(state): State<AppState>,
      mut request: Request,
      next: Next,
  ) -> Result<Response, ApiError>;
  ```

  Generate at least 32 random bytes with OS randomness, store only SHA-256 hex, and use the `cookie` crate to parse/render attributes. Authentication performs one joined lookup of the auth session and user. Add a test-only clock/token source to avoid sleeps and deterministic-secrets leakage into production constructors.

- [x] Split the router into public and protected branches. Public: `GET /health`, `POST /api/auth/login`, `POST /api/auth/logout`. Apply `require_user` to every other route, including `/api/me` and SSE. Change `/api/me` to `{id,name}` and map auth failures to stable codes.

- [x] Replace auth configuration with:

  ```rust
  pub session_ttl: Duration,              // SESSION_TTL_DAYS, default 30
  pub session_cookie_secure: bool,        // SESSION_COOKIE_SECURE, default false
  ```

  Remove `AuthMode`, `AUTH_MODE`, `AUTH_PROXY_TOKEN`, and the public-deployment proxy guard. Call migrations, interrupted-run recovery, expired-auth cleanup, and `seed_users` before building state/binding the listener. Bind address remains a deployment setting handled in the later V3 deployment plan; this task must not reintroduce proxy identity.

- [x] Run `cd backend && cargo test --test auth_scope --test user_seed --features e2e-test` and confirm it passes.

- [x] Commit: `git add backend/Cargo.toml backend/Cargo.lock backend/src/auth backend/src/api backend/src/application/mod.rs backend/src/config.rs backend/src/main.rs backend/src/bin/e2e-server.rs backend/tests backend/.env.example && git commit -m "feat: add persistent user login sessions"`.

### Task 3: Convert every domain repository and service to explicit user ownership

**Files:**
- Modify: `backend/src/db/repositories/{session,message,history,workspace,ingestion,simulation,task,dataset,analysis,comparison,storage_scope,artifact}.rs`
- Modify: `backend/src/application/{artifacts,chat,events,example_import,history,ingestion,model_patch,workspace,simulations,analysis,comparisons,visualization}.rs`
- Modify: `backend/src/api/{sessions,events,workspaces,ingestions,tasks,datasets,analysis,comparisons,examples,storage}.rs`
- Modify: `backend/src/tools/*.rs`
- Modify: `backend/tests/*.rs`

- [ ] First change the isolation tests to log in as two seeded users and enumerate every protected resource class: session, message, attachment/import, workspace, version, script/run once introduced, dataset rows/download, analysis, comparison, history, and storage usage. Add a direct-ID probe for each class and assert the same 404 body as a missing ID.

- [ ] Run `cd backend && cargo test --test auth_scope --features e2e-test` and retain the expected compile/test failures while old scopes remain.

- [ ] Replace repository signatures consistently:

  ```rust
  // old
  async fn find(&self, scope: &AccessScope, id: i32) -> Result<T, RepoError>;
  // V3
  async fn find(&self, user_id: &UserId, id: i32) -> Result<T, RepoError>;
  ```

  Session queries use `sessions.user_id = ?`. Every child-resource query joins to `sessions` and applies the same predicate. Artifact quota operations receive the trusted `storage_scope_id` from `CurrentUser`; they never resolve a client-supplied scope. Remove `Principal`, `AccessScope`, and `StorageScopeRef` from application requests, EventWriter, scheduler owner keys, tools, and handlers.

- [ ] Make scheduler ownership `UserId`, while preserving global/per-user concurrency semantics. Update stored SSE/event routing keys to `UserId`. Use `CurrentUser` extractor/extension in handlers and pass only `&current.id` plus trusted `current.storage_scope_id` downward.

- [ ] Run all backend tests: `cd backend && cargo test --features e2e-test`. Fix each former instance/proxy fixture by authenticating through the real login route; do not add a test-only production bypass.

- [ ] Search for forbidden V2 identity remnants:

  `rg -n 'AccessScope|StorageScopeRef|CurrentPrincipal|trusted_proxy|AUTH_MODE|AUTH_PROXY_TOKEN|x-zjudss-subject|x-zjudss-proxy-token' backend integration-tests frontend docs`

  Production code must have zero matches; historical spec/plan prose may retain explanatory matches.

- [ ] Commit: `git add backend/src backend/tests && git commit -m "refactor: scope domain data by fixed user"`.

### Task 4: Terminate SSE immediately on auth expiry or logout

**Files:**
- Modify: `backend/src/auth/session.rs`
- Modify: `backend/src/api/events.rs`
- Modify: `backend/src/application/events.rs`
- Add test: `backend/tests/auth_sse.rs`
- Modify: `backend/tests/sse_recovery.rs`

- [ ] Add failing paused-time tests proving: connection authentication can renew once; heartbeats do not update `last_seen_at_ms/expires_at_ms`; expiry ends the stream; logout of the same auth session ends it immediately; logout of a different browser session for the same user does not end it; Last-Event-ID recovery remains unchanged while authenticated.

- [ ] Run `cd backend && cargo test --test auth_sse --test sse_recovery --features e2e-test` and confirm failure.

- [ ] Add an `AuthSessionRevocations` broadcast keyed by `token_hash`. On logout, delete first and then broadcast. In the SSE loop, `select!` over domain events, heartbeat, and revocation; at each heartbeat perform a read-only `is_session_active(token_hash, now_ms)` check without renewal. End the stream quietly on revoke/expiry so the frontend can re-probe `/api/me`.

- [ ] Run the focused tests and then `cd backend && cargo test --features e2e-test`.

- [ ] Commit: `git add backend/src/auth/session.rs backend/src/api/events.rs backend/src/application/events.rs backend/tests/auth_sse.rs backend/tests/sse_recovery.rs && git commit -m "feat: revoke authenticated event streams"`.

### Task 5: Add the frontend auth state machine and sidebar user menu

**Files:**
- Add: `frontend/src/types/auth.ts`
- Modify: `frontend/src/types/index.ts`
- Modify: `frontend/src/api/client.ts`
- Modify: `frontend/src/api/events.ts`
- Add: `frontend/src/components/auth/LoginPage.tsx`
- Add: `frontend/src/components/auth/UserMenu.tsx`
- Modify: `frontend/src/components/history/HistorySidebar.tsx`
- Modify: `frontend/src/App.tsx`
- Add: `frontend/src/state/auth.ts`
- Add tests: `frontend/src/components/auth/LoginPage.test.tsx`
- Add tests: `frontend/src/state/auth.test.ts`
- Modify tests: `frontend/src/api/events.test.ts`, `frontend/src/test/smoke.test.tsx`, `frontend/src/release/semantic-assertions.test.ts`

- [ ] Write failing Vitest coverage for `loading | anonymous | authenticated`, keyboard login, duplicate-submit protection, unified credential error, successful state replacement, global 401 reset, user-menu name/ID/logout, mobile menu behavior, and SSE authentication failure stopping reconnect.

- [ ] Run `cd frontend && npm test -- --run` and confirm the new tests fail.

- [ ] Define the public client contract:

  ```ts
  export interface User { id: string; name: string }
  export type AuthState =
    | { status: 'loading' }
    | { status: 'anonymous' }
    | { status: 'authenticated'; user: User; epoch: number }

  export const login = (id: string, password: string) =>
    request<{user: User}>('/api/auth/login', { method: 'POST', body: JSON.stringify({id,password}) })
  export const logout = () => request<void>('/api/auth/logout', { method: 'POST' }, async () => undefined)
  export const getCurrentUser = () => request<User>('/api/me')
  ```

  Have the shared request client emit one `authentication-required` event on any protected 401. Do not persist credentials or user objects in localStorage.

- [ ] Refactor `App` so it renders no business hooks/SSE until authenticated. Mount the authenticated workspace under `key={`${user.id}:${epoch}`}` (or an equivalent child component boundary) so logout/login destroys all messages, drafts, workspaces, runs, datasets, selections, and timers. Change the navigation preference key to `zjudss:last-session:${user.id}`.

- [ ] Put `UserMenu` at the bottom of `HistorySidebar`, showing name and an avatar initial; its menu shows ID and logout. Preserve session rename/delete and the existing narrow-screen overlay. Use `BroadcastChannel('zjudss-auth')` for immediate same-origin logout UX; still rely on the server for authority.

- [ ] Run `cd frontend && npm test -- --run && npm run build`.

- [ ] Commit: `git add frontend/src && git commit -m "feat: add fixed-user login experience"`.

### Task 6: Convert browser fixtures and prove end-to-end isolation

**Files:**
- Modify: `integration-tests/e2e/fixtures.ts`
- Modify: `integration-tests/e2e/helpers/server.ts`
- Modify: `integration-tests/e2e/helpers/journeys.ts`
- Replace: `integration-tests/e2e/auth-isolation.spec.ts`
- Modify: all `integration-tests/e2e/*.spec.ts` callers that inject proxy headers
- Modify: `backend/src/bin/e2e-server.rs`
- Add fixture: `integration-tests/fixtures/init_users.csv` if deterministic e2e identities cannot use the embedded production fixture without revealing real IDs in test logs

- [ ] Change the browser fixture to obtain cookies through `POST /api/auth/login`; never inject identity headers. Provide `loginAs(id)` and a second isolated browser context. The e2e server must seed deterministic test users through the same `seed_users` path.

- [ ] Add browser journeys for persistent refresh, two users' direct-URL/search/resource isolation, logout/back-button denial, switching identities without stale paint, and a database reset invalidating an old cookie. Record successful responses for login/logout/me and every protected route required by the functional coverage gate.

- [ ] Run `cd frontend && npm run test:e2e -- auth-isolation.spec.ts` and fix only auth/fixture failures.

- [ ] Run `cd frontend && npm run test:e2e`.

- [ ] Commit: `git add integration-tests backend/src/bin/e2e-server.rs && git commit -m "test: cover v3 authenticated isolation"`.

### Task 7: Publish the V3 authentication contract and run release gates

**Files:**
- Modify: `docs/api-design.md`
- Modify: `docs/db-design.md`
- Modify: `docs/frontend-design.md`
- Modify: `docs/architecture.md`
- Modify: `docs/development-status.md`
- Move/replace: `docs/v2-deployment.md` → `docs/v3-deployment.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `.github/workflows/pr-functional-coverage.yml`
- Modify: `.github/workflows/release-real-deps.yml`
- Modify: `README.md` if it names V2 auth/runtime commands

- [ ] Update the API/schema/frontend diagrams and examples to only describe users, login cookies, user-owned sessions, and V3 reset deployment. Document the explicit risk that password equals ID and `SESSION_COOKIE_SECURE=true` is required behind public HTTPS. Record the embedded seed SHA-256 in candidate-release evidence without listing IDs.

- [ ] Update every renamed route/DTO/error and add behavioral catalog entries for seed initialization, invalid credentials, persistent login, logout, SSE revocation, and each cross-user resource class. Remove proxy-header feature entries rather than marking them manual or skipped.

- [ ] Run formatting and unit/integration gates:

  ```bash
  cd backend
  cargo fmt --all -- --check
  cargo test --features e2e-test
  cd ../frontend
  npm test -- --run
  npm run build
  npm run test:e2e
  cd ..
  ./scripts/check-functional-coverage --require-pr
  ```

- [ ] Run `git status --short` and `git diff --check`; verify `backend/resources/init_users.csv` remains the user's unstaged change and is absent from all task commits.

- [ ] Commit: `git add docs integration-tests/catalog .github README.md && git commit -m "docs: publish v3 authentication contract"`.
