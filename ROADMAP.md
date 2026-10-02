# Implementation roadmap

This roadmap preserves the full scope in docs/PRODUCT_SPEC.md. Checked implementation entries are not public release claims. Keep coding through the backlog after each tested, committed slice.

## First vertical slice — implemented and locally validated

- [x] Rust workspace and Tauri 2 / React desktop.
- [x] SQLite/PostgreSQL sessions, table/view discovery and table inspection.
- [x] Metadata storage, keychain separation, URL parsing, groups/favorites/environment/read-only labels.
- [x] SQL tabs, current statement/selection/all execution, formatting, completion, results and cancellation.
- [x] Bounded cursor transport, disk spool, virtualized page, column layout/copy/cell inspector.
- [x] Rust CSV/JSON/JSONL/SQL/Markdown export.
- [x] Workspace persistence, query history, saved queries, theme/settings, command palette.
- [x] Real packaged macOS workflow: saved connection, 10,000-row query, paging, cancellation and verified CSV export.
- [x] Cross-platform CI: macOS, Windows and Linux builds/tests; PostgreSQL and audit jobs (run 36995057710).
- [x] PostgreSQL 16 real-server contract validated locally and in CI.
- [x] Workspace save ordering/close flush, per-tab table inspector, automatic table query and bounded IPC pages.
- [ ] Backend benchmark history and release package validation.

## Next working slices

- [ ] Safe parameterized table editing, insert/delete/bulk changes with optimistic concurrency and transaction rollback.
- [ ] Streaming CSV/JSON/SQL imports and server-side table filters/sort/pagination.
- [ ] MySQL/MariaDB, pooling/reconnect, connection testing, timeouts and complete metadata.
- [ ] Verified TLS options/client certificates and SSH tunnels/bastion support.
- [ ] SQL Server, DuckDB and ClickHouse with actual integration services.
- [ ] Explain tree, ER diagrams with saved layouts, DDL/statistics/triggers/constraints.
- [ ] MongoDB document/aggregation/editing UI and Redis typed keys/TTL/explorer.
- [ ] CockroachDB/Redshift/TiDB compatibility verified against actual servers.
- [ ] Broader engines: Oracle, Cassandra/Scylla, Firebird, LibSQL, BigQuery, Snowflake, DynamoDB, Trino/Presto, SurrealDB and practical HANA support.

## Product and release gates

- [ ] Editor error locations, robust alias/column completion, query favorites/recent refinements and shortcut preferences.
- [ ] First-class simultaneous connection and transaction state; configurable production confirmations.
- [ ] Cold/warm interactive startup, process-tree memory, five connections, 100k rows, large schema, 100 tabs, scroll frames, query overhead/throughput/cancellation benchmark history.
- [ ] Address measured bottlenecks without relaxing targets; compare against other clients only with reproducible evidence.
- [ ] GitHub formatting/clippy/tests/lint/typecheck/audits, real database integration and desktop E2E.
- [ ] macOS ARM/Intel, Windows x64 and Linux x64 CI/package verification; signed/notarized artifacts where credentials permit.
- [ ] License inventory and audit, real screenshots, public release notes and downloadable packages.
- [ ] Signed updater configuration when a release distribution/key infrastructure exists.
- [ ] Driver loading/package isolation after measuring multi-driver footprint; safe extension model after core stability.
- [ ] Later XLSX/Parquet, backup/restore, additional platforms.

## Update policy and current evidence

Last updated: 2026-10-02. Current evidence is recorded in [docs/VALIDATION.md](docs/VALIDATION.md). Update this file in the same commit as every meaningful working change, recording completed behavior, validation and the next unfinished milestone.

Every meaningful working state is committed and pushed directly to main. Do not tag an incomplete or unverified application as a usable release.
