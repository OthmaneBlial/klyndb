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
