# 多研究类型意图规划 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Agent 将潮流、QSTS、短路、谐波和暂态自然语言请求转换成结合 active 模型与当前引擎合同的确定性研究计划，缺关键物理参数时先追问，只有 ready 计划可以创建相应 RSS 脚本。

**Architecture:** 新增 application 层 `StudyPlanningService`，用受控 tagged request、流式模型能力提取和纯策略评估返回四态 `StudyPlan`；新增只读 `plan_study` 工具并在请求内签发 ready plan ID。adapter 静态脚本校验暴露受管函数名，`create_script` 在持久化 Agent 研究脚本前验证计划 ID、函数、active 模型 hash 和引擎 snapshot，Prompt/skills 只负责自然语言归一化与基于缺失项追问。

**Tech Stack:** Rust 2024、Axum、rig 0.45、Serde、Arrow/Parquet 60、Diesel/SQLite、现有 Agent bundle/ScriptedAgent、Vitest/Playwright release lane

**Spec:** `docs/superpowers/specs/2026-10-25-study-intent-planning-design.md`

## Global Constraints

- 版本化模型保持固定 21 张表；不得保留服务器 DSS 路径或恢复 CSV 结果链路。
- 规划只读取当前用户、当前会话的 active 模型；草稿不作为可运行计划依据。
- 除静态潮流外，缺少关键物理参数必须返回 `needs_clarification`，不得从案例、历史或隐式默认补齐。
- 当前暂态能力只映射 `ds_motor_dynamics`；EMT、暂态稳定和逆变器控制暂态返回 `unsupported`。
- 所有函数名、arity 和结果名必须与本次启动的 `functions.json` snapshot 核对。
- 所有 tensoreval 调用仍只通过 `backend/src/adapters/`；规划读取 Parquet 不启动子进程。
- `create_script` 只保存脚本；SimulationRun 仍只能由用户点击脚本卡片按钮创建。
- LLM 和前端只接收稳定资源 ID、opaque plan ID、受控 spec 和有界摘要，不接触路径、命令或任意 JSON。
- Agent bundle 升级为 `v3.2.0`，公共工具从 17 个增加为 18 个，manifest/hash/注册表/文档/功能目录同步更新。
- 直接在 `master` 工作，不创建 worktree；提交时排除无关的用户改动。

## Review Focus

- 部分开断线路、完全开断开关、多绕组变压器和重复母线对不会把网架错误归类为 Radial（Task 2 facts tests）。
- 大模型使用投影流式扫描而不是 Agent 分页或一次性 JSON 序列化，读取失败时不降级猜测（Task 2 streaming/error tests）。
- `Some([])` 的无事件电机动态与 `None` 的未说明事件严格区分，且非空 storage 明确拒绝（Task 1 transient policy tests）。
- ready plan 在 active version 或 engine snapshot 变化后不能授权旧脚本，且拒绝发生在脚本持久化前（Task 4 stale-proof tests）。
- 通用非研究 RSS、手工上传脚本和既有脚本不因新增 study proof 回归（Task 4 compatibility tests）。

---

### Task 1: Define bounded study requests and pure readiness policy

**Files:**
- Create: `backend/src/application/study_planning/mod.rs`
- Create: `backend/src/application/study_planning/contracts.rs`
- Create: `backend/src/application/study_planning/policy.rs`
- Modify: `backend/src/application/mod.rs`
- Test: `backend/src/application/study_planning/policy.rs`

**Interfaces:**
- Produces: `StudyRequest::{StaticPowerFlow,Qsts,Fault,Harmonic,Transient}` with serde tagged `studyType` and the exact bounded fields from the spec.
- Produces: `SolverPreference::{Auto,Radial,Newton}`, `StudyStatus::{Ready,NeedsClarification,MissingModelData,Unsupported}`, `MissingParameter`, `ModelStudyFacts`, `StudyFunctionContract`, `StudyPlan` and `ReadyStudyPlan`.
- Produces: `evaluate(request: &StudyRequest, facts: &ModelStudyFacts, functions: &StudyFunctionCatalog) -> StudyPlan` as a pure function.
- Produces: `StudyFunctionCatalog::from_value(&serde_json::Value) -> Result<Self, StudyPlanningError>` and exact lookup of names, `complete_arity` or fixed `arity`, default result and results.
- Consumed by Tasks 2–4: public contracts and `ReadyStudyPlan` retain active version/hash, engine snapshot ID, normalized request, selected function, arity and result names.

- [ ] **Step 1: Write failing policy tests for every study family**

  Add literal assertions for `static_power_flow_is_ready_and_selects_radial_or_newton`, `qsts_reports_all_missing_time_and_profile_fields`, `fault_requires_explicit_location_kind_terminals_impedance_and_ideal_source_consent`, `harmonic_distinguishes_missing_spectrum_from_missing_h_max`, and `transient_requires_subtype_and_only_supports_motor_dynamics`. Include the Review Focus case proving `events=None` needs clarification while `events=Some([])` is explicit.

- [ ] **Step 2: Run the policy tests and verify RED**

  Run: `cd backend && cargo test application::study_planning::policy::tests --lib`

  Expected: FAIL because the module and contracts do not exist.

- [ ] **Step 3: Implement request validation and four-state policy**

  Implement the interfaces above. Bound results/monitor lists/fault cases/events to 64, QSTS inline curve points to 1,000, text identifiers to 256 bytes, `number` to `1..=100_000`, and require finite positive time values. Empty result lists select `bus_phase,summary` for static power flow and the catalog default for other studies; non-empty results must be unique catalog members.

- [ ] **Step 4: Add catalog drift and boundary tests**

  Assert missing function, wrong `complete_arity`/`arity`, duplicate/unknown result, non-finite value, excessive arrays and incompatible explicit solver produce the exact approved status/error rather than a guessed plan. Use a fixture `functions_json` containing all seven governed function names and arities 21/33/22/28.

- [ ] **Step 5: Run Task 1 tests and verify GREEN**

  Run: `cd backend && cargo test application::study_planning --lib`

  Expected: all policy and catalog tests pass.

- [ ] **Step 6: Commit Task 1**

  ```bash
  git add backend/src/application/study_planning backend/src/application/mod.rs
  git commit -m "feat: define deterministic study planning policy"
  ```

### Task 2: Extract active-model study facts with user-scoped streaming reads

**Files:**
- Create: `backend/src/application/study_planning/facts.rs`
- Modify: `backend/src/application/study_planning/mod.rs`
- Modify: `backend/src/application/dataframe_reader.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/tests/common/mod.rs`
- Create: `backend/tests/study_planning.rs`

**Interfaces:**
- Produces: `StudyPlanningService::new(VersionRepo, ArtifactService, Arc<EngineResourceSnapshot>)`.
- Produces: `StudyPlanningService::plan(&self, user: &CurrentUser, session_id: i32, request: StudyRequest) -> Result<StudyPlan, StudyPlanningError>`.
- Produces: crate-private projected batch streaming helper on `ParquetDataFrameReader` that opens only an owned artifact and invokes a blocking visitor without serializing rows to JSON.
- Produces: `facts::extract_active_model_facts(...) -> Result<ModelStudyFacts, StudyPlanningError>` with topology/device/reference facts and short-circuit source-equivalent evidence from the active 21-table snapshot.
- Consumes: Task 1 `evaluate` and `StudyFunctionCatalog`; parses `engine_snapshot.functions_json` on service construction and includes `engine_snapshot.id` in plans.

- [ ] **Step 1: Write failing active-scope and topology integration tests**

  In `backend/tests/study_planning.rs`, publish controlled Parquet fixtures and assert: no active model returns `missing_model_data`; a single-source tree is Radial; a loop, parallel edge, second voltage source, `generator.execution_mode=CONSTANT_PV`, or `ibr.port_model=GFM` selects Newton; another user's session is indistinguishable from missing. Add open-edge and multi-winding cases from Review Focus.

- [ ] **Step 2: Run the new integration tests and verify RED**

  Run: `cd backend && cargo test --test study_planning --features e2e-test`

  Expected: FAIL because `StudyPlanningService` and the streaming fact extractor do not exist.

- [ ] **Step 3: Add the owned projected streaming primitive**

  Add a crate-private API taking `&CurrentUser`, `DataFrameReadSource`, projected columns and a `Send + 'static` batch visitor. Resolve ownership before `spawn_blocking`, iterate `ParquetRecordBatchReaderBuilder` batches without the public 500-row/64-KiB limits, and map all failures to safe `StudyPlanningError` values.

- [ ] **Step 4: Implement complete model fact extraction and topology analysis**

  Resolve workspace ownership through `VersionRepo` once, require the active version, index table metadata by canonical name, and scan only required columns. Build the phase-agnostic energized graph, referenced bus/terminal sets, spectrum references and PV/GFM flags; never expose raw file handles or paths outside the application layer.

- [ ] **Step 5: Add unreadable/schema-drift and large-stream tests**

  Assert a malformed required schema and unreadable owned artifact return stable `study_model_read_failed`; create more than 500 projected rows and prove topology uses all rows rather than the Agent/page limit.

- [ ] **Step 6: Run Task 2 tests and verify GREEN**

  Run:

  ```bash
  cd backend
  cargo test application::study_planning --lib
  cargo test --test study_planning --features e2e-test
  cargo test --test model_dataframe_reads --features e2e-test
  ```

  Expected: all pass and existing model table reads retain their public limits.

- [ ] **Step 7: Commit Task 2**

  ```bash
  git add backend/src/application/study_planning backend/src/application/dataframe_reader.rs backend/src/application/mod.rs backend/tests/study_planning.rs backend/tests/common/mod.rs
  git commit -m "feat: derive study capabilities from active models"
  ```

### Task 3: Expose `plan_study` and issue request-local ready plan IDs

**Files:**
- Create: `backend/src/tools/plan_study.rs`
- Modify: `backend/src/tools/mod.rs`
- Modify: `backend/src/application/chat.rs`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/tools/support.rs`
- Modify: `backend/resources/agent/v3/manifest.json`
- Modify: `backend/tests/tool_manifest.rs`
- Modify: `backend/tests/workspace_tools.rs`
- Modify: `backend/tests/contract_manifest.rs`

**Interfaces:**
- Produces: public tool `plan_study` with `Args { request: StudyRequest }`; description requires `study-planning` skill and states that only `ready` returns `studyPlanId`.
- Produces: `ReadyStudyPlanRegistry::{insert,get}` stored as `Arc<Mutex<_>>` in each `ToolServices`, keyed by `study-plan:<sha256>` over request message ID plus canonical ready plan.
- Produces: `DomainToolAdapter<T>` used by the registry macro so all 18 tools preserve `DomainToolError.code`, safe model feedback and `retryable=false` through Rig dispatch.
- Adds: `StudyPlanningService` to `AppState`, `ChatService` and `ToolServices` without adding a REST route.
- Preserves: all existing 17 tool names and appends `plan_study` as the eighteenth manifest/registry entry.
- Consumed by Task 4: lookup returns the full immutable `ReadyStudyPlan`, not an LLM-supplied reconstruction.

- [ ] **Step 1: Write failing tool contract tests**

  Extend manifest/registry tests to expect exactly 18 tools and add tool calls proving: missing skill returns model-visible `skill_required` rather than `the tool failed`; non-ready results contain no `studyPlanId`; ready static power flow returns an opaque ID; IDs cannot be resolved from another request registry.

- [ ] **Step 2: Run focused tests and verify RED**

  Run:

  ```bash
  cd backend
  cargo test --test tool_manifest
  cargo test --test workspace_tools plan_study -- --nocapture
  cargo test --test contract_manifest
  ```

  Expected: FAIL because the tool, registry and manifest entry do not exist.

- [ ] **Step 3: Implement service wiring, tool schema and ready registry**

  Construct one `StudyPlanningService` in `AppState`; clone it into each ChatService/ToolServices. Require `study-planning`, call the service with the bound principal/session, serialize the bounded plan, and insert only `Ready` into the request-local registry. Use canonical JSON plus SHA-256 for the opaque ID; do not persist non-ready plans. Wrap every registered tool once in the registry macro with `DomainToolAdapter`, preserving names, schemas, annotations and successful outputs.

- [ ] **Step 4: Run Task 3 tests and verify GREEN**

  Run:

  ```bash
  cd backend
  cargo test --test tool_manifest
  cargo test --test workspace_tools --features e2e-test
  cargo test --test contract_manifest
  ```

  Expected: all pass with an 18-tool public contract and request-local plan IDs.

- [ ] **Step 5: Commit Task 3**

  ```bash
  git add backend/src/tools/plan_study.rs backend/src/tools/mod.rs backend/src/tools/support.rs backend/src/application/chat.rs backend/src/application/mod.rs backend/resources/agent/v3/manifest.json backend/tests/tool_manifest.rs backend/tests/workspace_tools.rs backend/tests/contract_manifest.rs
  git commit -m "feat: add the study planning agent tool"
  ```

### Task 4: Require matching ready plans for Agent-generated study scripts

**Files:**
- Modify: `backend/src/adapters/script.rs`
- Modify: `backend/src/adapters/mod.rs`
- Modify: `backend/src/application/scripts.rs`
- Modify: `backend/src/tools/create_script.rs`
- Modify: `backend/src/tools/support.rs`
- Modify: `backend/tests/script_validation_adapter.rs`
- Modify: `backend/tests/scripts.rs`
- Modify: `backend/tests/workspace_tools.rs`

**Interfaces:**
- Extends: `StaticScriptValidation` with `called_functions: Vec<String>` serialized as `calledFunctions`, detected only from actual call expressions for the seven governed names.
- Extends: `create_script` args with optional `studyPlanId`; generic non-study RSS may omit it.
- Extends: internal `CreateScriptRequest` with `study_plan: Option<ReadyStudyPlan>`; manual/upload call sites pass `None`.
- Produces: pre-persistence errors `study_plan_required`, `study_plan_not_found`, `study_plan_stale` and `study_plan_function_mismatch`.
- Records: `study-planning` and `rss-generation` skill refs plus plan hash, model hash, snapshot ID and chosen function in the existing script audit JSON.

- [ ] **Step 1: Write failing validator and script-proof tests**

  Assert exact function detection does not match comments, string literals or names such as `my_ds_fault_study`; test governed Agent script without ID, unknown ID, wrong function and changed active version all fail before script/message/artifact persistence. Test a matching ready plan succeeds. Add Review Focus compatibility cases for generic Agent RSS, manual upload and existing script detail.

- [ ] **Step 2: Run focused tests and verify RED**

  Run:

  ```bash
  cd backend
  cargo test --test script_validation_adapter
  cargo test --test scripts study_plan -- --nocapture
  cargo test --test workspace_tools create_script -- --nocapture
  ```

  Expected: FAIL because validation has no function list and create_script accepts no plan ID.

- [ ] **Step 3: Extend adapter validation with governed call detection**

  Parse identifier-plus-opening-parenthesis call sites in `backend/src/adapters/script.rs`, ignoring comments and quoted literals using the existing RSS lexical constraints. Return sorted unique governed names without changing required alias/output behavior.

- [ ] **Step 4: Enforce proof before committing Agent study scripts**

  Resolve the opaque ID only from the request-local registry, compare function, current active version ID/hash and engine snapshot, then pass the immutable plan into `ScriptService`. After adapter validation and before artifact commit, require the proof only when `source == Agent` and `called_functions` contains a governed function; attach audit refs on success.

- [ ] **Step 5: Run Task 4 and regression tests and verify GREEN**

  Run:

  ```bash
  cd backend
  cargo test --test script_validation_adapter
  cargo test --test scripts --features e2e-test
  cargo test --test workspace_tools --features e2e-test
  cargo test --test simulation_runs --features e2e-test
  ```

  Expected: all pass; rejected attempts create no script artifact/message and explicit run behavior is unchanged.

- [ ] **Step 6: Commit Task 4**

  ```bash
  git add backend/src/adapters/script.rs backend/src/adapters/mod.rs backend/src/application/scripts.rs backend/src/tools/create_script.rs backend/src/tools/support.rs backend/tests/script_validation_adapter.rs backend/tests/scripts.rs backend/tests/workspace_tools.rs
  git commit -m "feat: bind study scripts to ready plans"
  ```

### Task 5: Teach the Agent the unified planning workflow and publish bundle v3.2.0

**Files:**
- Modify: `backend/resources/agent/v3/system.md`
- Modify: `backend/resources/agent/v3/skills/study-planning.md`
- Modify: `backend/resources/agent/v3/skills/rss-generation.md`
- Modify: `backend/resources/agent/v3/skill-index.json`
- Modify: `backend/resources/agent/v3/manifest.json`
- Modify: `backend/src/application/mod.rs`
- Modify: `backend/src/config.rs`
- Modify: `backend/tests/agent_knowledge.rs`
- Modify: `backend/tests/app_bootstrap.rs`
- Modify: `backend/tests/scripted_agent.rs`
- Modify: `integration-tests/release/real-dependencies.spec.ts`
- Modify: `integration-tests/catalog/features.yaml`
- Modify: `docs/agent-prompt-skills-design.md`
- Modify: `docs/integration-testing-design.md`
- Modify: `docs/development-status.md`

**Interfaces:**
- Consumes: Tasks 1–4 `plan_study` states, ready plan ID and create_script proof.
- Produces: bundle `v3.2.0` with Chinese/English triggers for all five study families and an unchanged eight-turn cap.
- Produces: conversation contract `inspect_workspace → load study-planning → plan_study → clarify OR load rss-generation/docs/create_script`.
- Produces: deterministic and real-provider behavior coverage for short ready power flow, incomplete QSTS/fault/harmonic/transient, supported motor dynamics and unsupported EMT.

- [ ] **Step 1: Write failing Scripted Agent behavior tests**

  Add scenarios with exact user messages: `计算潮流`, `短路计算`, `做一天时序模拟`, `谐波分析`, `暂态仿真`, `模拟电机启动暂态2秒，步长0.01秒，无外部事件`, and `做EMT暂态仿真`. Assert static power flow follows a ready plan and creates the exact planned function; the four incomplete requests return one assistant clarification containing all tool-reported missing fields and no script; supported motor dynamics creates `ds_motor_dynamics` only when model facts permit; EMT reports unsupported; no scenario creates a run.

- [ ] **Step 2: Run the behavior and bundle tests and verify RED**

  Run:

  ```bash
  cd backend
  cargo test --test scripted_agent study -- --nocapture
  cargo test --test agent_knowledge
  cargo test --test app_bootstrap
  ```

  Expected: FAIL because Prompt/skills still allow free script generation and bundle version is v3.1.0.

- [ ] **Step 3: Update system Prompt, skills and skill index**

  Encode only the semantic mapping, mandatory planner flow, one-message clarification rule, current transient boundary and plan-ID handoff. Keep function details behind current engine docs and plan output. Add the five study trigger families and require `plan_study` from `study-planning`; retain `rss-generation` gating for `create_script`.

- [ ] **Step 4: Publish and hash bundle v3.2.0**

  Update active version literals in application/config/tests, compute changed skill and system SHA-256 values, update `skill-index.json` and `manifest.json`, and keep the allowed tool list exactly equal to the 18-tool registry.

- [ ] **Step 5: Run focused bundle and behavior tests and verify GREEN**

  Run:

  ```bash
  cd backend
  cargo test --test scripted_agent
  cargo test --test agent_knowledge
  cargo test --test app_bootstrap
  cargo test --test tool_manifest
  cargo test --test contract_manifest
  ```

  Expected: all pass with bundle v3.2.0 and the approved clarify/ready branches.

- [ ] **Step 6: Update release evaluation, docs and functional traceability**

  Add `AGENT-STUDY-PLANNING-001` with the new tool/skill/tests to `features.yaml`. In the real-dependency release journey, submit the exact short prompts above, validate plan/tool traces and confirm no script on incomplete/unsupported branches; for ready branches inspect the saved script before the existing explicit Run click. Update the three current design/status documents with 18 tools, four statuses, active-only planning, supported functions and the 21-table limitation.

- [ ] **Step 7: Run deterministic full verification**

  Run:

  ```bash
  cd backend
  cargo fmt --all -- --check
  cargo clippy --all-targets --all-features -- -D warnings
  cargo test --features e2e-test
  cd ../frontend
  npm test -- --run
  npm run build
  npm run test:e2e
  cd ..
  ./scripts/check-functional-coverage --require-pr
  git diff --check
  ```

  Expected: zero fmt/clippy/test failures, frontend build and Playwright pass, PR functional coverage is 100%, and diff check is silent.

- [ ] **Step 8: Run the real-dependency candidate gate when credentials and side effects are approved**

  Run:

  ```bash
  ./scripts/test-release-integration
  ./scripts/check-functional-coverage --require-release
  ```

  Expected: all five natural-language families follow the planned branch with the real provider and current tensoreval/resources; no skip or fallback. If external approval or credentials are unavailable, report this gate as not run rather than claiming it passed.

- [ ] **Step 9: Commit Task 5**

  ```bash
  git add backend/resources/agent/v3 backend/src/application/mod.rs backend/src/config.rs backend/tests/agent_knowledge.rs backend/tests/app_bootstrap.rs backend/tests/scripted_agent.rs integration-tests/release/real-dependencies.spec.ts integration-tests/catalog/features.yaml docs/agent-prompt-skills-design.md docs/integration-testing-design.md docs/development-status.md docs/superpowers/specs/2026-10-25-study-intent-planning-design.md docs/superpowers/plans/2026-10-25-study-intent-planning.md
  git commit -m "release: publish unified study intent planning"
  ```
