# Engine resource fixture

`engine_resources.rs` builds the catalog, docs, RSS and bundle tree in a temporary directory so each test can alter hashes, paths and capabilities independently. This directory records that the fixture is intentional; no test reads the developer's `.env`.
