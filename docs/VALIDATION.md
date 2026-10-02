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
