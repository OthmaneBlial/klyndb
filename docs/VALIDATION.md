# Validation log

## First native slice — 2026-10-02, b82e9c3

- `cargo test`: five focused Rust tests passed, including a real SQLite file, type preservation, cancellation and disk paging. The separate PostgreSQL contract is ignored by default and was explicitly run against a disposable local PostgreSQL 16 instance.
- PostgreSQL contract passed: DDL/DML, schema/PK inspection, large integer text/NULL, empty and multiple result sets, row cap, server cancellation and session reuse.
- Frontend typecheck, lint, two statement-selection tests and production build passed.
- Workspace Clippy with warnings denied and rustfmt passed.
- `cargo audit`: zero vulnerability entries; two upstream Linux warnings documented in SECURITY.md. `npm audit`: zero reported vulnerabilities. `cargo deny check licenses`: passed.
- `npm run tauri build -- --debug --bundles app`: produced a real macOS `.app` (about 53 MiB, debug build). It launched from embedded `tauri://localhost` assets, independently of Vite.
- Native UI test used a clearly named local validation SQLite database: create saved connection, connect, enter a three-statement DDL/insert/select batch, return 10,000 real rows, preserve NULL, switch to page 2 (row 501), refresh schema and discover the new table.
- Native export dialog wrote a CSV through Rust. A separate filesystem check verified its header, 10,000 records and final ID 10000. The validation artifact was moved into ignored artifacts/.
- A recursive SQLite query was cancelled from the real UI; the session remained responsive. Cmd+K opened the real command palette. The initial cancellation message was `interrupted`; normalization is being corrected.

This proves the described workflows on the local macOS debug bundle, not all production workflows or release quality. No mock IPC or demo backend was used. GitHub Actions run 36995057710 passed macOS, Windows and Linux builds, PostgreSQL integration and audits. These are build/test results; releases, all-driver compatibility and desktop performance gates remain pending.

## Workbench reliability follow-up — 2026-10-02

- Focused core regression passed: 10,000-row disk pages, 8 MiB IPC page bound and normalized explicit cancellation; oversized pages fail clearly while a smaller page succeeds.
- Frontend lint/typecheck/two tests/build and workspace Clippy passed; rebuilt the native macOS debug bundle (53.34 MiB).
- Native UI: opening a table immediately returned 10,000 real rows. Switching to the unrelated SQL tab removed its Structure inspector.
- Changed SQL and immediately closed the window; relaunch restored the exact new text. Cmd+K focused the command search field.
- This commit updates ROADMAP.md and makes same-commit roadmap maintenance an explicit contribution rule.

## Safe table editing slice — 2026-10-02

- Rust tests and Clippy passed; PostgreSQL 16 real-server contract passed with bound update/delete values, conflict detection, whole-batch rollback and preservation of an outer manual transaction.
- SQLite regression covered bound text containing SQL-like syntax, exact signed 64-bit integer keys, binary values, NULL updates, generated-column guards, no-PK rejection, conflicts after a preceding insert and manual rollback.
- PostgreSQL idle/active/failed transaction-state checks passed, including recovery with ROLLBACK. Core production-write confirmation is enforced in Rust.
- Frontend lint/typecheck/tests/build passed; native macOS debug bundle rebuilt (54.10 MiB).
- Real native UI: opened a table, staged an update and an insert, reviewed their concrete old/new values, applied both in one batch and saw the re-read values. Staged deletion of the inserted test row required confirmation and was applied successfully.
- Executing BEGIN showed an active-transaction indicator. Executing ROLLBACK removed it. Keyboard navigation opened the row editor.
- Filesystem verification independently checked 10,000 records, absence of the deleted test row and the persisted updated value. Automated desktop E2E and Windows/Linux native editing remain pending.

## Validation policy change — 2026-10-02

By explicit owner request, GitHub Actions was disabled at repository level (`enabled: false`). Both workflow definitions were removed; active native-package run 36997559834 was cancelled. CI run 36997475264 for 0277fa8 had already completed successfully on all three OSes before this instruction. These historical results do not authorize future GitHub execution. Current validation uses `scripts/check.sh` locally; remote package verification is incomplete.

`./scripts/check.sh` passed locally with the disposable PostgreSQL URL: locked dependency install, frontend checks/build/audit, workspace Rust checks/tests, real PostgreSQL integration, native build and license audit. The two previously documented upstream Rust warnings remain visible. The package run is terminal `cancelled`; no queued or active GitHub runs remained at verification.

## Backend streaming baseline — 2026-10-02

`cargo build --release -p klyndb-core --example stream_bench` succeeded. `scripts/bench_stream.py` ran ten real samples (five each at 100k / 1m rows), with correct retained counts and readable final pages. The median 1m-row run took 2,837 ms at 352,393 rows/sec and 13.50 MiB peak process RSS. This covers the backend result flow only. Machine/toolchain/commit/lock hash and every raw sample are retained under benchmarks/history; desktop memory/startup/scrolling and comparisons remain unmeasured.

## MySQL/MariaDB vertical slice — 2026-10-02

- The same real-server contract passed separately against disposable MySQL 8.4.11 and MariaDB 13.0.2 servers on loopback, with separate data directories. It covers table/view DDL, PK/generated/index/foreign-key metadata, quoted identifiers, large unsigned integers, exact decimals, NULL/binary values, non-UTF-8 byte preservation, empty/multiple results, row limits, transaction state, repeated cancellation, session reuse after server errors and read-only write/DDL/batch rejection. Default TLS rejected the untrusted/plaintext fixtures; these tests explicitly opt out only for those local services.
- Native macOS debug bundle built (61.60 MiB). The real UI saved/connected a MySQL connection, discovered and opened a real table using driver-native backticks, returned binary/NULL cells, and displayed columns, indexes and DDL. BEGIN displayed the transaction indicator; two result sets preserved a 20-digit integer and a large exact decimal. ROLLBACK plus SLEEP was cancelled from the UI; the transaction indicator cleared and a subsequent SELECT 42 succeeded.
- MySQL/MariaDB staged grid edits remain unavailable (`edit_rows=false`); the SQL editor executes real writes. This is a working query/metadata slice, not a complete-driver or all-platform claim.
- A local frontend test exposed a CodeMirror timing bug: an incomplete parser tree could fall back to executing the whole document. Current-statement execution now forces bounded full parsing and returns no SQL when parsing is unavailable; explicit selection/all commands remain separate. Four focused frontend tests pass, including end-of-file/trailing whitespace and unavailable-parser cases.
- New locked dependencies were audited; license inventory/notices include 839 packages, preserving all prior notice sections. Cargo audit still reports only the two documented upstream warnings. No GitHub workflow was run; repository Actions remains disabled.

`./scripts/check.sh` passed again after the parser correction, with four frontend tests, workspace checks, real PostgreSQL and MariaDB contracts, native build and audits. The MySQL contract also passed separately. Logs are kept in ignored local artifacts; GitHub Actions stayed disabled.

The final native bundle was relaunched and reconnected. With two statements in the editor and the cursor at the end, Run returned only `current_statement = 42`; the earlier SELECT was not executed. Both servers passed the additional legacy-encoding preservation contract; focused Clippy/rustfmt passed after that change.

## Public repository presentation — 2026-10-02

README positioning explicitly describes Klyndb as a free, open-source alternative to DBeaver. The original SVG cover uses Klyndb branding; the unedited JPEG is captured from the actual packaged macOS application with a disposable SQLite validation database and a real MySQL connection. It contains synthetic test records and no user data. Current engine support, preview limitations and backend-only benchmark scope remain explicit. GitHub Actions stays disabled.

## MySQL/MariaDB staged editing — 2026-10-02

- Real MySQL 8.4.11 and MariaDB 13.0.2 edit contracts cover bound inserts/updates/deletes, adjacent maximum unsigned keys, exact large decimals, binary/NULL, native JSON and MariaDB text aliases, generated-column protection, ordinary timestamp defaults, legacy-encoded columns, case/trailing-space conflicts caused by another connection, no-op updates, default/auto-ID inserts and quoted names. Stale batches roll back preceding inserts. Manual transactions and `autocommit=0` preserve uncommitted work; read-only connections, views and MyISAM staged writes are rejected. Non-strict conversion warnings abort writes; changed session encodings fail before modifying data.
- Both MySQL and MariaDB deadline contracts held a row lock from another session for longer than the 60-second client edit limit. The batch returned an error at about 60 seconds, its preceding insert was rolled back, original data remained, and transaction-state/session reuse succeeded. A real trigger writing to a MyISAM audit table demonstrated an incomplete rollback: the InnoDB row was rolled back, the audit effect survived, and the driver explicitly closed the connection instead of claiming atomic success.
- Full local `scripts/check.sh` passed with PostgreSQL and MariaDB integration. Final additional deadline/rollback checks and native UI evidence are recorded below. GitHub Actions remained disabled.

Final workspace Clippy with warnings denied, rustfmt and all three MySQL contracts passed independently on both servers; MariaDB's deadline/rollback run also passed. Locked license inventory remains 839 packages. The rebuilt macOS debug bundle is 61.82 MiB. Native UI verified staged update+insert review/application, confirmed deletion, pending editing inside BEGIN, ROLLBACK restoring original data, and rejection of a stale post-rollback row. A separate native MySQL client confirmed that uncommitted edits were invisible and the temporary inserted row was removed. Local evidence is retained under ignored artifacts/. Automated desktop E2E and Windows/Linux native editing are still pending.

## Isolated connection testing — 2026-10-02

The core regression checks an existing SQLite transaction survives a test, saved metadata/session identity remain unchanged, and a missing SQLite file is never created. A non-database file fails without changing its bytes. Optional real PostgreSQL/MySQL/MariaDB URLs exercise the same driver-opening path. A loopback TCP peer that accepts but never answers verifies the complete handshake fails at the 10-second limit. PostgreSQL session ownership now covers read-only initialization so failure/cancellation closes the worker.

Native macOS UI: tested the saved MySQL URL successfully, changed only the draft to an unused loopback port and saw a connection error, then closed the draft. The original session executed SELECT 42 and still reported its actual open transaction, which was explicitly rolled back. The form displays a real test result/error and disables fields while working. Validation logs remain in ignored artifacts/.

Full `scripts/check.sh` passed with real PostgreSQL/MariaDB URLs, including the 10-second handshake test and all three MariaDB edit/query contracts. The isolated test also passed separately with PostgreSQL/MySQL URLs. The final macOS debug bundle rebuilt at 61.85 MiB. A local ad-hoc signature was applied and `codesign --verify --deep --strict` passed; this is not Developer ID signing or notarization. The final visual recheck of the color-field width correction could not complete because the native automation tool repeatedly returned `cgWindowNotFound`, including after relaunch/reset. The functional native workflow above had passed before that CSS correction. Process sampling reached the native AppKit event loop; it does not substitute for a visible-window check.

## Executable-comment validation — 2026-10-02

A real MariaDB regression reproduced a read-only bypass: `SELECT 1; /*M! COMMIT */; /*M! UPDATE ... */` was classified as read-only, committed the driver's protective transaction, and changed the disposable fixture. An independent native client confirmed the write. The shared validator now retains executable comments as tokens and rejects both MySQL and MariaDB forms, including version guards, before query submission. The MySQL dialect's existing parser behavior is preserved for ordinary SQL; tokenization occurs once, and the original SQL is still used for execution.

The regression passes after the fix on both disposable MySQL 8.4.11 and MariaDB 13.0.2 servers; an independent connection confirms the protected data stays unchanged. Unit checks accept ordinary comments, optimizer hints and SQL literals containing comment markers, while rejecting executable comments. SQLite's ordinary comment behavior remains unchanged. Policy references: [MySQL comments](https://dev.mysql.com/doc/refman/8.4/en/comments.html) and [MariaDB comments](https://mariadb.com/docs/server/reference/sql-statements/comment-syntax). No external source implementation was reused.

Full local `scripts/check.sh` passed with PostgreSQL and MariaDB integrations, including all three MariaDB contracts, native build and audits. The MySQL query contract passed separately. The locked dependency inventory is unchanged; the same two upstream audit warnings remain documented. GitHub Actions is still disabled. Logs: ignored `artifacts/local-ci-executable-comments.log` and `artifacts/mysql-executable-comments.log`.

## Native execution plans — 2026-10-02

The real core contract passed on SQLite, PostgreSQL 16, MySQL 8.4.11 and MariaDB 13.0.2: original single-statement handling with comments/Unicode, native estimated plans, estimated UPDATE/DELETE leaving data unchanged, runtime metrics when supported, mandatory confirmation, caller-owned transaction rollback, cancellation, timeout and session reuse. Read-only SELECT plans passed on all engines; runtime plans were rejected. MariaDB can reject DML planning in a read-only transaction; the server's restriction remains intact. PostgreSQL parenthesized ANALYZE options now require confirmation rather than being classified as estimated output. Malformed/cyclic/deep/oversized trees fail explicitly.

MySQL accepted a simple UPDATE with EXPLAIN ANALYZE but returned estimated output without changing the fixture. Its runtime button therefore permits only the verified read-only query slice; broader runtime DML remains pending. PostgreSQL/MariaDB runtime UPDATE changed the fixture inside an explicit BEGIN, and ROLLBACK restored it. No automatic rollback is claimed.

Full local `scripts/check.sh` passed with PostgreSQL/MariaDB, including the new core plan contract and all previous contracts. The final plan contract passed separately against MySQL and verifies server warning capture. After lazy rendering of collapsed tree children, frontend lint/typecheck/four tests and the native build passed again. License regeneration retained exactly 839 locked packages and unchanged notices; audit warnings are the same two documented upstream entries. Logs remain in ignored artifacts (`local-ci-plans.log`, `plans-mysql-final.log`, `native-build-plans.log`). Actions stayed disabled.

The macOS debug app rebuilt at 62.40 MiB; local ad-hoc signing and strict codesign verification passed, without Developer ID or notarization. Fresh native launch succeeded and resolved the earlier automation window failure. Native MySQL UI verified an estimated JSON tree, expansion of collapsed branches, server Note 1003, raw JSON, clipboard-copy success, concrete ANALYZE confirmation and a runtime tree with actual times/rows/loops. Cancelling ANALYZE SELECT SLEEP(30) reported Query cancelled with no successful plan; the same session then returned SELECT 42. Native SQLite displayed a primary-key SEARCH operator and ID/parent without timing/cost claims or an Analyze button. The connection color field is now visibly within its form row; that previously blocked visual check passed. The unedited native plan screenshot is under docs/assets/explain-macos.jpg. Native PostgreSQL/MariaDB and Windows/Linux plan UI checks remain pending.

## Wide-result streaming export — 2026-10-02

A real SQLite result with 40 rows containing 300,000-character values reproduced an export failure: the previous native export reused a 500-row IPC page and hit its 8 MiB safety bound. Export now reads the completed private result spool through a single native cursor, deserializing one row at a time. UI page bounds and the 512 MiB retained-result quota stay enforced. The regression failed before the fix and passes after it; it independently reads every exported CSV field, verifies all 40 rows and checks export of the separate second result set. Native file replacement still occurs only after a successful write and sync.

Full local `scripts/check.sh` passed with real PostgreSQL and MariaDB, including the existing cancellation/edit-timeout contracts, frontend checks, workspace tests, native build and audits. The same two documented upstream Rust audit warnings remain; the lockfile changes only internal workspace dependency edges. GitHub Actions remains disabled. Log: ignored `artifacts/local-ci-wide-export.log`.

The rebuilt 62.40 MiB macOS debug bundle passed local ad-hoc signing/strict signature verification. Native UI ran the wide query, retained the explicit UI page-limit message, then successfully exported all 40 rows through the Save dialog. A separate Python CSV reader verified the header, every complete 300,000-character field and the final row in the 12,000,162-byte file. The file stays under ignored artifacts/. This validates the local macOS workflow, without claiming Developer ID signing, notarization or Windows/Linux export UI coverage.

## Native CSV import foundation — 2026-10-02

The real bounded CSV-to-driver contract passed on SQLite, PostgreSQL 16, MySQL 8.4.11 and MariaDB 13.0.2. It imports 1,201 quoted/multiline records, independently queries the stored count and last value, and verifies a bad numeric record after the preceding batches rolls back the whole import. Duplicate keys, explicit producer errors and dropped producers also undo earlier batches. An explicit completion message is required before commit.

Manual transaction checks preserve earlier uncommitted work while rolling back only failed imports. A concurrent COMMIT on the same session waits until the import has ended. Cancellation while an input producer is still alive and cancellation during a real independent row lock both allow rollback and subsequent session reuse. Read-only, generated-column, non-insert and MyISAM destination guards pass. A trigger writing into MyISAM demonstrates the server limitation: InnoDB inserts are undone, the audit effect survives, and the session is closed with an unconfirmed-rollback warning. An independent observer verifies both counts.

The parser/mapping contract passes for UTF-8 BOM, fragmented one-byte input, multiline/doubled quotes, CRLF, malformed quotes, invalid UTF-8, field-count/record-size limits, immutable snapshots, duplicate/generated mappings and large numeric/JSON tokens. No new external package versions were added; csv-core, UUID and Tokio were already locked dependencies. Native database coercion remains in force. Desktop file picker/IPC jobs, mapping/progress UI and production confirmations are still pending; this is a native foundation, not a completed user-facing import feature. Logs are retained under ignored artifacts (`import-parser-final.log`, `import-final-maria.log`, `import-final-mysql.log`).

The idle PostgreSQL import cancellation test exposed a late CancelQuery packet reaching the next statement. Native import cancellation now sends an interrupt only while database work is active, not while waiting for input or rolling back. Ordinary query interruption drains already-completed results without sending a packet, and otherwise requires a synchronized native cancellation response before reuse. Uncertain termination closes the worker; core job errors preserve the connection-closure warning even when cancellation or a timeout also occurs.

A separate disposable PostgreSQL regression reproduced cancellation hanging with a full result channel. Shared result delivery now observes cancellation and terminates/drains native work when the consumer disappears. The regression verifies bounded termination and a subsequent SELECT 42 on the same session. Before/after logs: ignored `postgres-backpressure-before.log` and `postgres-backpressure-after.log`.

The same regression also covers a consumer closing while PostgreSQL is executing pg_sleep with no delivered rows; the stream loop now observes receiver closure directly. A separate test terminates only its own disposable read-only PostgreSQL backend to force protective ROLLBACK failure. It reproduced the earlier error masking the closed session; cleanup now preserves the original error and explicitly reports connection closure. The pre-fix failure is retained in ignored `postgres-cleanup-before.log`.

Full local `scripts/check.sh` passed after the final cleanup correction, with real PostgreSQL/MariaDB URLs, workspace parsing/import/plan/export tests, all three PostgreSQL contracts and all three MariaDB contracts, native build and audits. The standalone MySQL import contract also passed. Locked license regeneration retained exactly 839 packages with unchanged notices. The same two documented upstream Rust audit warnings remain. GitHub Actions stayed disabled. Final CI log: ignored `artifacts/local-ci-import-foundation.log`.

The final macOS debug bundle rebuilt at 62.93 MiB, passed local ad-hoc signing and strict signature verification, and relaunched successfully. Native UI restored the workspace, reconnected the disposable SQLite database and returned import_foundation_validation = 42 in the real grid. Import controls are intentionally not exposed yet. This is a local macOS check, without Developer ID signing, notarization or Windows/Linux package claims. Native build log: ignored `artifacts/native-build-import-foundation.log`.
