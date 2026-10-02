# Implementation roadmap

This roadmap preserves the full scope in docs/PRODUCT_SPEC.md. Checked implementation entries are not public release claims. Keep coding through the backlog after each tested, committed slice.

**Current validation policy:** GitHub Actions is disabled at the owner's request (2026-10-02). Run `./scripts/check.sh` locally for every working milestone. Keep remote Actions disabled and do not add workflow triggers without a new explicit instruction. Historical CI results below describe completed runs before this policy change.

## First vertical slice — implemented and locally validated

- [x] Rust workspace and Tauri 2 / React desktop.
- [x] SQLite/PostgreSQL sessions, table/view discovery and table inspection.
- [x] Metadata storage, keychain separation, URL parsing, groups/favorites/environment/read-only labels.
- [x] SQL tabs, current statement/selection/all execution, formatting, completion, results and cancellation.
- [x] Bounded cursor transport, disk spool, virtualized page, column layout/copy/cell inspector.
- [x] Rust CSV/JSON/JSONL/SQL/Markdown export.
- [x] Stream exports directly from the native result spool, one row at a time, independently of the 8 MiB UI page bound; wide-result regression verifies all rows and result-set selection.
- [x] Workspace persistence, query history, saved queries, theme/settings, command palette.
- [x] Real packaged macOS workflow: saved connection, 10,000-row query, paging, cancellation and verified CSV export.
- [x] Cross-platform CI: macOS, Windows and Linux builds/tests; PostgreSQL and audit jobs (run 36995057710).
- [x] PostgreSQL 16 real-server contract validated locally and in CI.
- [x] Workspace save ordering/close flush, per-tab table inspector, automatic table query and bounded IPC pages.
- [x] Reproducible release backend streaming baseline: five runs each at 100k / 1m rows, disk pages, throughput, first-row timing and native peak RSS.
- [ ] Release package validation locally on each platform. A local optimized macOS arm64 candidate passes archive/signature/architecture/resource checks; native release acceptance and the other target platforms remain pending. See [release guide](docs/RELEASES.md).

## Next working slices

- [x] Parameterized SQLite/PostgreSQL insert/update/delete batches; optimistic old-value checks, PK guards, generated-column protection and savepoint rollback. Native Rust checks passed, including PostgreSQL 16.
- [x] Real native macOS staged update/insert batch, review, confirmed deletion and transaction-state indicator; production writes enforced in Rust.
- [ ] Automated desktop E2E for the editing workflow and equivalent native Windows/Linux behavior.
- [x] Native CSV import foundation: private immutable snapshot, bounded strict UTF-8 parsing, clipped preview, typed column mapping and whole-stream transaction/savepoint rollback.
- [x] CSV desktop file picker, opaque source/job IDs, mapping/preview, production confirmation, deadlines, progress/cancel and result refresh. Core contracts pass on all four engines; packaged macOS SQLite file selection, mapping/preview, append and grid refresh verified.
- [x] Streaming JSON imports: standard object arrays and explicit typed export arrays, bounded records, exact Rust number tokens, native NULL semantics and shared mapping/job/transaction pipeline. Real SQLite/PostgreSQL/MySQL/MariaDB core contracts and full configured local CI pass, including late-error rollback and manual-transaction export roundtrips. Packaged macOS SQLite standard/typed JSON picker, preview, mapping, import and refresh are verified, with independent stored-value and CSV-export checks. Initial debug/release process attribution was ambiguous, so this does not establish acceptance of an exact optimized release artifact. Windows/Linux and automated desktop import E2E remain pending.
- [ ] Streaming SQL file imports.
- [x] Server-side table filters/sort/pagination for SQLite/PostgreSQL/MySQL/MariaDB: real-engine contracts and full local CI pass; packaged macOS SQLite initial/next/previous pages, numeric column filtering, descending sort and generated SQL synchronization verified after native UI access recovered. Windows/Linux UI checks remain pending.
- [ ] Equivalent Windows/Linux import workflows and automated desktop import E2E.
- [x] Publish the original Klyndb showcase/docs at OthmaneBlial.github.io/klyndb/: portable static pages, real screenshots, local fonts, mobile layout and snippet copy; live HTTPS verified. Snippet-copy check is included in local CI. The table-browsing, constraint/trigger and relationship-diagram updates are published and verified on the live landing/docs pages; custom-CA and client-identity and connection-timeout instructions are also live (Pages 7c99058), with HTTP byte matching and actual Chrome verification. The CSV/JSON import guide is live (Pages 09b6554), with both HTML pages matching source bytes and actual Chrome verification. The saved-credential timeout guide is also live (Pages f1faed4), with exact HTTPS bytes and actual Chrome verification. README and repository homepage link to the live site; GitHub Actions CI remains disabled.
- [x] MySQL/MariaDB native connection/query/metadata slice, exact numeric/binary/NULL cells, multiple results, row cap, cancellation, read-only validation, native identifier quoting and actual transaction state. Real MySQL 8.4.11 / MariaDB 13.0.2 contracts and native macOS MySQL workflow passed.
- [x] MySQL/MariaDB InnoDB staged insert/update/delete; exact bound values, optimistic conflicts, savepoint rollback, manual/autocommit-disabled transactions, conversion-warning guards and table-level editability.
- [x] MySQL/MariaDB real 60-second locked-row edit timeout and incomplete-trigger rollback contracts; unconfirmed rollback closes the session.
- [x] Native macOS MySQL staged review/update/insert/delete, manual transaction rollback and stale-row rejection.
- [x] Isolated connection tests before saving; no metadata/credential writes or SQLite file creation, existing sessions/transactions preserved. A 10-second end-to-end deadline covers stalled handshakes; invalid SQLite files fail on connect.
- [ ] Reconnect and broader metadata.
- [x] Configurable connection setup deadlines for PostgreSQL/MySQL/MariaDB: shared validated 1–300-second URL/UI control, default 10; real eleven-second delayed handshakes pass for Test connection and Connect, one-second stalls preserve existing transactions, and full local CI passes. The setup deadline now includes saved database/TLS/SSH credential reads and the serialized session gate. A dedicated, bounded credential worker returns an actionable timeout without accumulating blocked readers; the current-thread regression, scoped native Keychain contract and configured real-server local checks pass. The refreshed embedded macOS debug bundle rebuilt and passed ad-hoc signature verification. Native timeout interaction remains unverified: window control cannot attach to the confirmed live app or Chrome. Windows/Linux interactions remain pending.
- [x] Verified TLS transport controls and native PEM/DER custom CA picker for PostgreSQL/MySQL/MariaDB; real encrypted-session, chain/hostname rejection, metadata, cancellation and reuse contracts pass. The packaged macOS PostgreSQL picker/test/save/reconnect workflow and encryption query are verified; backend and local CI evidence is recorded in docs/VALIDATION.md.
- [x] PKCS#12 TLS client identities for PostgreSQL/MySQL/MariaDB with bounded native file loading, separate OS keychain passwords and real certificate-required query/metadata/cancel/reconnect contracts. Packaged macOS PostgreSQL native PKCS#12 selection, test/save/query and fresh-process keychain reconnect are verified; native MySQL/MariaDB and Windows/Linux checks remain pending.
- [x] Single-bastion SSH tunnels for PostgreSQL/MySQL/MariaDB: pinned host keys before credentials, password/agent/private-key authentication, separate scoped keychain secrets and preserved database TLS hostnames. Full configured local CI and real three-engine OpenSSH/mTLS contracts pass. The provider's destructive read-buffer flush was corrected; independent 2 MiB duplex and simultaneous 3 MiB half-close regressions pass. Packaged macOS pin rejection, native encrypted-key selection, isolated test, save/connect, encryption query and fresh-process credential reconnect are verified; the guide and live website are updated.
- [ ] Further SSH/TLS work: native Windows/Linux and desktop cancellation checks, idle-CPU and larger concurrent-load profiling, multi-hop/proxy/MFA/known_hosts UX and additional TLS identity formats.
- [ ] SQL Server, DuckDB and ClickHouse with actual integration services.
- [x] Native estimated plans and confirmed runtime analysis: PostgreSQL JSON, MySQL JSON/TREE, MariaDB JSON and SQLite QUERY PLAN; bounded Rust tree decoding, raw output, native metrics and server warnings. Real-engine cancellation/timeouts and explicit transaction behavior verified.
- [x] Native macOS MySQL estimated/runtime tree, raw output/copy, confirmation, server messages, cancellation and session reuse; SQLite QUERY PLAN without invented runtime metrics.
- [ ] PostgreSQL/MariaDB native desktop plan checks, Windows/Linux plan workflows and MySQL runtime DML beyond the verified SELECT slice.
- [x] Table Structure constraint/trigger inspection: PostgreSQL native definitions/firing state, MySQL/MariaDB names/types/timing/body and SQLite native trigger definitions with constraint DDL. Real four-engine contracts and UI rendering check pass; rebuilt macOS SQLite Structure view, expanded trigger definition and constraint DDL, plus MySQL constraint table/DDL verified. Windows/Linux UI validation remains pending.
- [x] Native SQL relationship diagrams: composite/self-referencing foreign keys, columns/PKs/FKs, manual/grid layout, pan/zoom/Fit, local saved state and native SVG export. Four-engine contracts and local CI pass; packaged macOS SQLite drag/pan/keyboard/zoom/Fit/auto-layout, save/reopen, search/subset and independently parsed SVG verified.
- [ ] Large-catalog diagram performance checks, native diagram interactions on PostgreSQL/MySQL/MariaDB and Windows/Linux, additional named layouts and routed layout refinements.
- [ ] PostgreSQL table DDL, statistics, trigger functions, structured SQLite constraint extraction and safe schema editing.
- [ ] MongoDB document/aggregation/editing UI and Redis typed keys/TTL/explorer.
- [ ] CockroachDB/Redshift/TiDB compatibility verified against actual servers.
- [ ] Broader engines: Oracle, Cassandra/Scylla, Firebird, LibSQL, BigQuery, Snowflake, DynamoDB, Trino/Presto, SurrealDB and practical HANA support.

## Product and release gates

- [x] Reject executable MySQL/MariaDB comments in shared SQL validation, preventing MariaDB comments from escaping read-only transactions or bypassing destructive-query checks. Regression reproduced the write on a disposable MariaDB server before the fix.
- [x] Current-statement execution waits for a complete parser tree and fails closed when unavailable; regression protects against accidental whole-file execution.
- [ ] Editor error locations, robust alias/column completion, query favorites/recent refinements and shortcut preferences.
- [x] Actual transaction state after queries and successful/failed edit batches; failed PostgreSQL transactions require ROLLBACK, closed sessions show an unavailable state.
- [x] PostgreSQL cancellation during result backpressure or consumer loss; drain completed responses before cancellation and close sessions when interruption cannot be synchronized. Preserve connection-closure warnings in core job errors.
- [ ] Refine simultaneous connection lifecycle and configurable production confirmations.
- [ ] Cold/warm interactive startup, process-tree memory, five connections, 100k rows, large schema, 100 tabs, scroll frames, query overhead/throughput/cancellation benchmark history.
- [ ] Address measured bottlenecks without relaxing targets; compare against other clients only with reproducible evidence.
- [x] Local CI entry point for formatting/clippy/tests/lint/typecheck/audits/native build and configured real PostgreSQL / MySQL / MariaDB integration.
- [ ] Extend local real-database integration and desktop E2E as new drivers are added.
- [ ] macOS ARM/Intel, Windows x64 and Linux x64 local package verification; signed/notarized artifacts where credentials permit.
- [x] GitHub Actions disabled; active native-package run cancelled. Remote package validation remains incomplete.
- [x] Locked dependency license inventory/audit and real native macOS screenshot in the README. Notice collection now retains LICENCE, NOTICE/NOTICES, UNLICENSE and OFL variants and fails on undecodable notice text; the real 921-package inventory regenerates deterministically and the retention regression runs in local CI.
- [x] GitHub positioning as a free, open-source alternative to DBeaver; redesigned SVG cover and star banner, emoji feature highlights, real native screenshot, verified feature matrix and preview limits.
- [ ] Public release notes and downloadable, validated native packages.
- [ ] Signed updater configuration when a release distribution/key infrastructure exists.
- [ ] Driver loading/package isolation after measuring multi-driver footprint; safe extension model after core stability.
- [ ] Later XLSX/Parquet, backup/restore, additional platforms.

## Update policy and current evidence

Last updated: 2026-10-02. Current evidence is recorded in [docs/VALIDATION.md](docs/VALIDATION.md). Update this file in the same commit as every meaningful working change, recording completed behavior, validation and the next unfinished milestone.

Every meaningful working state is committed and pushed directly to main. Do not tag an incomplete or unverified application as a usable release.
