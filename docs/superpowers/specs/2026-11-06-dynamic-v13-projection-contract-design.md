# Dynamic V13 Projection Contract Design

## Intent

Accept the published eig-rs Dynamic V13 contract without weakening case safety. A bundle may expose
more native study results than its managed entry selects: `manifest.study.results` is the native
result inventory, while `manifest.projection.results` is the ordered managed subset used to build
`para.result` and validate managed bindings.

## Contract

- Enabled projection profiles use
  `profile.resultMap(manifest.projection.results)` as their selector value source.
- Every managed projection result must occur once in `manifest.study.results`, in the same order.
- Every managed source must map through the profile `bundleResultMap`, and its target and
  `result_<source>` binding must remain exact.
- Source/projector/target/managed-entry/status/stable-diagnostic matching remains fail closed.
- Native results omitted from the managed subset are metadata only; they do not become browser or
  LLM-controlled ECharts options, paths, commands, SQL, or arbitrary scripts.

The Dynamic V13 GFL case therefore remains valid with 16 native results and 15 managed results.
Malformed, unknown, duplicate, reordered, or incorrectly bound managed results remain invalid.

## Release evidence

`scripts/test-release-engine` must remove its exact previous report after preflight and before the
test process starts, so a failed candidate cannot leave an older report looking current. A successful
run must regenerate evidence from the current binary and immutable resource snapshot.

## Documentation

Current deployment and integration documents describe the distinction between native and managed
results. Historical 2026-10-30 counts stay labeled as historical; new counts are recorded only after
the real candidate gate succeeds.

## Non-goals

- Do not add compatibility for case projection schema v1.
- Do not loosen profile/facade input, result, schema, hash, file, or projector-output validation.
- Do not change REST/SSE DTOs, database schema, Agent tools, frontend behavior, or ECharts contracts.
- Do not edit eig-rs or its sparrowzz resource tree from zjudss.
