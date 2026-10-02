# Pinned native ClickHouse client

Source: klickhouse 0.15.3, crates.io archive SHA-256 `43df4e6356e65204d50730a3699b49a0ad345a947b725a0aaf7544f691c8e98a`; upstream commit `0e514f93fb07073dbf86f7b7a180e4656f117244`. Original source and MIT / Apache-2.0 licenses are retained.

Klyndb removes payloads from native packet/server-error logs in `src/client.rs` and `src/internal_client_in.rs`; server messages and rows may contain credentials. Klyndb changes `src/client.rs` to use a read-only SELECT handshake probe instead of altering date parsing settings; native readonly=1 users can connect. Klyndb changes `src/block.rs` to reject duplicate result-column names before IndexMap overwrites values. Queries must use distinct aliases. This closes the session with an actionable error instead of silently losing data. The driver contract exercises rejection and a fresh reconnect. The packaged crate does not include license files; the originals here are from the exact upstream commit. `src/values/date.rs` gates the serde-only ParseError import to avoid an unused-import warning. Cargo.toml removes absent example/test targets, without changing dependency requirements.

Remove the path patch when an upstream release rejects duplicate columns or preserves their names and values.
