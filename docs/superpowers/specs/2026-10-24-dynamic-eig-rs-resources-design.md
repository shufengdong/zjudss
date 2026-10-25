# Dynamic eig-rs Resources Design

## Intent

V3 always uses the current `TENSOREVAL_BIN` and `SPARROWZZ_DIR` supplied by the operator. `SPARROWZZ_DIR`
points directly to `eig-rs/sparrowzz`. The application does
not pin or compare eig-rs/sparrowzz commits, source-dirty state, catalog hashes, documentation hashes,
example-list hashes, or an exact example count. An incompatible or broken current engine is reported
as an explicit startup, import, validation, or execution error; the application does not silently fall
back to a stub or an older snapshot.

## Goals

- Accept the latest readable eig-rs tree without regenerating an embedded resource manifest.
- Keep the V3 adapter interface contract: named Parquet inputs, `--result-dir`, Parquet output, both
  model-import profiles, and script validation.
- Discover documentation and examples from the current tree and calculate an immutable runtime
  snapshot ID from the bytes actually loaded.
- Preserve path containment, manifest self-consistency, schema validation, stable example IDs, and
  immutable runtime resource snapshots.
- Report current-engine failures without weakening validation or rewriting the eig-rs worktree.

## Non-goals

- No automatic checkout, pull, clone, build, reset, or cleanup of eig-rs.
- No compatibility shim for old catalog schemas.
- No production fallback to `StubEngine`.
- No guarantee that every future eig-rs change is usable; incompatibility is a visible error.

## Approaches considered

1. **Dynamic runtime snapshot (selected).** Read and validate the current tree, then hash the loaded
   bytes for provenance. This follows latest eig-rs while retaining deterministic behavior inside a
   running server process.
2. Remove only the commit pin but retain embedded resource hashes. This still rejects the latest tree
   whenever docs, catalogs, or examples change, so it does not meet the requirement.
3. Regenerate and commit the manifest for every eig-rs update. This remains release-time pinning and
   creates recurring synchronization work, so it is rejected.

## Runtime discovery and validation

Startup orchestration first asks the adapter to probe `TENSOREVAL_BIN`, then passes the validated
capability/feature set into `EngineResourceRegistry::load(SPARROWZZ_DIR, capabilities)`. Resource code
never starts a subprocess. The registry performs one startup scan:

- Require `dss/docs/catalog/dataframes.json` and `functions.json` below `SPARROWZZ_DIR`.
- Parse both catalogs and require a schema version in a code-owned
  `SUPPORTED_CATALOG_SCHEMA_VERSIONS` set. A newer unknown schema fails with
  `engine_contract_mismatch` and a readable diagnostic; both catalogs also undergo their existing
  required-shape validation.
- Recursively discover regular UTF-8 Markdown files below `SPARROWZZ_DIR/dss/docs`, excluding symlinks and
  escaping paths, with bounded file count and total bytes. Catalog JSON files remain separately parsed.
- Discover `SPARROWZZ_DIR/dss/rssbundle/**/*.rssbundle/manifest.json` files in sorted portable-path order;
  top-level `.rss` files outside the bundle tree are not cases.
- Bundle file hashes declared by each current bundle manifest remain a self-consistency check. A broken
  bundle is listed as non-importable with diagnostics rather than changing another example.
- Resolve manifest-declared bundle files only inside `SPARROWZZ_DIR`; external, missing, symlink, and
  escaping dependencies make that example non-importable.
- The adapter probe must exercise named Parquet input/output, static DSS import, single-Solve import,
  and script validation rather than inferring support from the same help flags. Bundle feature names are
  compared with the feature set returned by the adapter. The binary and resource tree are not
  version-compared; later adapter failures are returned with their stable error details.

The registry hashes catalog bytes, every indexed document path/content, bundle manifest bytes, every
example path/content, and normalized diagnostics with a versioned, length-prefixed canonical encoding
into `engineResourceSnapshot`. During publish it re-reads and verifies each source hash, then atomically
copies exact docs, catalogs, and case input trees to
`DATA_DIR/system/engine-resources/<snapshotId>`. Case imports read only this published tree, never the
live eig-rs worktree. Existing destinations are verified against their manifest before reuse. A source
file changing during scan/publish fails startup with `engine_resource_changed_during_scan`. A process
never changes snapshots after startup; picking up a newer eig-rs tree requires a restart.

Resource limits are code-owned release constants: at most 512 Markdown documents, 4 MiB per document,
64 MiB total documentation, 2,048 examples, 10,000 files and 2 GiB per example, and dependency depth 64.
Exceeding one fails with `engine_resource_limit_exceeded`. An empty example set is a valid runtime state;
the real release gate separately requires at least one bundle and one importable example.

## Agent bundle and provenance

The Agent V3 manifest continues to hash its own system prompt, skill index, and skill files. Remove
`eigRsCommit`, `sparrowzzCommit`, `sourceDirty`, embedded catalog/docs/example hashes, and
`examples-source.json`. Keep `engineContractVersion` as the application/adapter protocol identifier,
not an eig-rs release identifier.

Model versions, runs, analysis runs, and comparisons continue to store the protocol-level
`engineContractVersion`; this change does not claim that they currently persist a resource snapshot or
binary hash, so no database migration is introduced.
Case application continues to submit the runtime `engineResourceSnapshot`, preventing a stale browser
selection from being applied after a server restart. Actual scripts/runs retain their existing artifact,
model, Agent bundle, skill, and engine-contract provenance.

## Errors and observability

- Missing/unreadable required paths: `engine_resource_io` or `engine_resource_unavailable`.
- Unsafe global docs/catalog paths or symlinks: startup `engine_resource_path_escape`.
- Invalid/unsupported catalogs: `catalog_invalid` or `engine_contract_mismatch`.
- Invalid, missing, escaping, symlinked, or hash-inconsistent bundle
  dependencies: the individual example remains in the internal/Agent registry as non-importable with
  path-redacted diagnostics; the public case API keeps its existing modelizable-only filter.
- Duplicate normalized paths or stable IDs: startup error because ownership is ambiguous.
- Missing engine capabilities: existing `engine_*_unsupported` startup errors.
- Runtime import/validation/execution failures: existing structured adapter errors; no fallback or retry
  against an older engine.

Bundle discovery never executes RSS. It reads only `manifest.json` and its declared files, validates safe
relative paths and hashes, and marks only the affected example non-importable when validation fails.

Startup logs and release reports record the computed snapshot ID, catalog schema, discovered/importable/
non-importable counts, and binary hash. They do not claim an eig-rs commit. The hash is observation evidence
only. Before every spawn, `ProcessEngine` checks only that the configured path is still a regular file;
replacing the executable takes effect on the next call without a restart.

## Tests and documentation

- Rewrite engine-resource tests around dynamic discovery, deterministic runtime hashes, path/size
  bounds, schema failures, bundle self-consistency, and snapshot changes when current bytes change.
- Remove exact `16/68/84` release assertions; require a non-empty bundle set and at least one importable
  example, then report discovered/importable/non-importable counts.
- Run the local real gate against the configured current `.env`; if the current eig-rs fails, preserve
  and report the failure instead of checking out a pinned revision.
- Change the candidate-release workflow to check out and build the default branch of eig-rs at workflow
  runtime. Remove engine artifact run ID and expected SHA inputs; the resulting binary hash and runtime
  resource snapshot remain evidence, not acceptance pins.
- Update Agent bundle tests, release semantic assertions, feature catalog, AGENTS.md, architecture,
  Agent/resource, deployment, integration-testing, and development-status documents in the same cut.

## Reset and migration

No database migration is required. The server must be restarted to load a newer eig-rs snapshot. Existing
published resources keep their stored provenance and remain readable; new work uses the newly computed
runtime snapshot.
