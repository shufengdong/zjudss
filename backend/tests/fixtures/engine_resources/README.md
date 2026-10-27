# Engine resource fixture

`engine_resources.rs` builds the docs, RSS and bundle tree in a temporary directory so each test can alter hashes, paths and capabilities independently. `catalog/` freezes the authoritative eig-rs catalog audited for V4 Task 2 (`functions=2`, `study_facades=1`, `dataframes=3`, `dtype_grammar=1`). Unit tests mutate copies; the separate real-tree test checks the current `SPARROWZZ_DIR`-compatible checkout. No test reads the developer's `.env`.
