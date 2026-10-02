# Architecture

The desktop is a Tauri 2 system-WebView shell. React represents the workspace; Rust owns database sessions, SQL validation, local persistence, cancellation and export.

| Module | Responsibility |
| --- | --- |
| `driver-api` | Session contract, capabilities, schema metadata and lossless cell values |
| `drivers/sqlite` | rusqlite on blocking workers; persistent connection and VM cancellation |
| `drivers/postgres` | tokio-postgres native protocol, verified native TLS, server cancellation |
| `query` | Dialect parsing and AST-based destructive-query checks |
| `connections` | SQLite application-state migration, metadata validation, URL secret extraction and OS credential store |
| `core` | Session/job lifecycle, timeout and disk-backed result spool |
| `export` | Streaming formats, independent of database engines and UI |
| `apps/desktop` | Tauri commands, native file dialogs and React workbench |

Drivers initialize on connection. No session opens during app startup. Each SQLite connection serializes access on its own mutex and runs on Tokio blocking workers. PostgreSQL serializes user SQL on its session to preserve transaction order. Connections remain independent.

Result flow: database cursor → four-slot Tokio channel → temporary SQLite spool → typed IPC page → virtualized DOM. Each batch has at most 256 rows and flushes at about 256 KiB; a single row above 8 MiB is rejected explicitly. The frontend holds a 500-row page, not the complete result. Metadata is polled while the query runs. A cached query is capped at 512 MiB of serialized rows; at most 32 result jobs and 100 statements per batch are retained. Closing a tab cancels/releases its job. Results disappear when the app closes; editor text and connection IDs persist.

Row limits apply per result set. PostgreSQL drains rows beyond the display limit so the connection and later statements remain usable; cancellation stops the server query. SQLite stops stepping a result at its limit. Multi-statement SQL is passed in its original form, including comments and vendor syntax; the AST is used for validation only.

SQLite state uses `user_version` migrations, parameter binding and WAL. Passwords never enter connection metadata. Keychain failure is surfaced and has no plaintext fallback. Queries/history can themselves contain sensitive literals; local history can be cleared.

## Add a driver

1. Create a crate under `crates/drivers` and implement `Session` from `driver-api`.
2. Report only working capabilities. Stream `Columns`, bounded `Rows`, then `Complete` for every statement, including empty results.
3. Implement cancellation and real table/column/index/foreign-key metadata. Preserve NULL, binary values and large integers.
4. Add a connector arm in `core`, connection validation in `connections`, and the installed engine in the form/editor dialect selector. The UI should otherwise use capabilities.
5. Run a contract against a real disposable database: connect, schema, DDL/DML, multi-results, NULL/types, row cap, cancellation, reconnect and disconnect. Add the compatibility row with explicit evidence.

Driver feature flags/process isolation are planned when compiled drivers or vendor libraries justify them. There are no untrusted native plugins or arbitrary shell commands.

## Table changes

Rust binds editing values separately from SQL. Only base tables are editable; schema metadata is re-read inside the edit transaction. Update/delete requires a non-NULL primary key and compares the original column values. Every batch runs in a savepoint and rolls back on a conflict or error. The driver commits only when it created the outer transaction; manual transactions remain open. The core requires confirmation for deletion and all production edits, and rejects read-only edits independently of UI controls.

SQLite uses native value codecs and generated-column metadata from `table_xinfo`. PostgreSQL binds text values, casts through trusted server type metadata, and compares original server text. Drivers report actual idle/active/failed transaction state after SQL; the UI never infers it by looking for a BEGIN keyword. PostgreSQL's state probe uses a short savepoint because tokio-postgres does not expose ReadyForQuery transaction status.

The behavior follows the independently implemented [SQLite savepoint](https://www.sqlite.org/lang_savepoint.html) and [PostgreSQL savepoint](https://www.postgresql.org/docs/current/sql-savepoint.html) semantics. For a new driver, implement safe change batches and transaction state, or return an explicit unsupported error with `edit_rows=false`.
