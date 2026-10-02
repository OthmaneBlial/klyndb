# Architecture

The desktop is a Tauri 2 system-WebView shell. React represents the workspace; Rust owns database sessions, SQL validation, local persistence, cancellation and export.

| Module | Responsibility |
| --- | --- |
| `driver-api` | Session contract, capabilities, schema metadata and lossless cell values |
| `drivers/sqlite` | rusqlite on blocking workers; persistent connection and VM cancellation |
| `drivers/postgres` | tokio-postgres native protocol, verified native TLS, server cancellation |
| `drivers/mysql` | mysql_async native protocol, dedicated user session, lazy cancellation connection, verified native TLS |
| `query` | Dialect parsing and AST-based destructive-query checks |
| `connections` | SQLite application-state migration, metadata validation, URL secret extraction and OS credential store |
| `core` | Session/job lifecycle, timeout and disk-backed result spool |
| `export` | Streaming formats, independent of database engines and UI |
| `apps/desktop` | Tauri commands, native file dialogs and React workbench |

Drivers initialize on connection. No session opens during app startup. Each SQLite connection serializes access on its own mutex and runs on Tokio blocking workers. PostgreSQL serializes user SQL on its session to preserve transaction order. MySQL/MariaDB also keeps a dedicated user session; its separate one-connection lazy pool sends KILL QUERY for cancellation. The running query future is retained until protocol termination, with a three-second fallback that closes the session if cancellation cannot be confirmed. Connections remain independent.

Result flow: database cursor → four-slot Tokio channel → temporary SQLite spool → typed IPC page → virtualized DOM. Each batch has at most 256 rows and flushes at about 256 KiB; a single row above 8 MiB is rejected explicitly. The frontend holds a 500-row page, not the complete result. Metadata is polled while the query runs. A cached query is capped at 512 MiB of serialized rows; at most 32 result jobs and 100 statements per batch are retained. Closing a tab cancels/releases its job. Results disappear when the app closes; editor text and connection IDs persist.

Row limits apply per result set. PostgreSQL and MySQL/MariaDB drain rows beyond the display limit so the connection and later statements remain usable; cancellation stops the server query. SQLite stops stepping a result at its limit. Multi-statement SQL is passed in its original form, including comments and vendor syntax; the AST is used for validation only.

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

SQLite uses native value codecs and generated-column metadata from `table_xinfo`. PostgreSQL binds text values, casts through trusted server type metadata, and compares original server text. MySQL/MariaDB refreshes transaction status with a harmless SELECT and uses native protocol flags rather than MariaDB-only SQL variables. Drivers report actual idle/active/failed transaction state after SQL; the UI never infers it by looking for a BEGIN keyword. PostgreSQL's state probe uses a short savepoint because tokio-postgres does not expose ReadyForQuery transaction status.

The behavior follows the independently implemented [SQLite savepoint](https://www.sqlite.org/lang_savepoint.html) [PostgreSQL savepoint](https://www.postgresql.org/docs/current/sql-savepoint.html), and [MySQL savepoint](https://dev.mysql.com/doc/refman/8.4/en/savepoint.html) semantics. For a new driver, implement safe change batches and transaction state, or return an explicit unsupported error with `edit_rows=false`.

MySQL/MariaDB edits use native bound parameters, exact unsigned integers/decimal text, and full binary original-value predicates. A metadata lock protects the InnoDB/base-table check and column whitelist. A UUID savepoint preserves user work; autocommit-disabled sessions stay pending. Conversion and rollback warnings fail explicitly. The edit deadline retains the same protocol future and stops before committing; once COMMIT starts, its acknowledgement is drained without KILL QUERY. Unconfirmed rollback/interruption closes the session. The desktop refreshes actual transaction state after successful or failed changes, including an unavailable-state indicator if the session closed.

[MySQL rollback semantics](https://dev.mysql.com/doc/refman/8.4/en/commit.html) cannot undo writes to nontransactional tables, including trigger side effects. Klyndb surfaces those server warnings. Staged edits require a UTF-8 client/connection/results session to avoid silently reinterpreting bound text; native legacy-encoded columns remain usable with UTF-8 session transport.

Connection testing shares the native driver-opening path, opens an isolated read-only session, probes the connection and disconnects it. It never saves metadata/passwords or creates SQLite files. Normal opens and tests have a 10-second total deadline, including handshake; tests also cover their probe/close. SQLite reads schema-version metadata on open to reject invalid database files. PostgreSQL constructs its owned session before read-only initialization so cancellation/errors abort its worker through Drop. Existing user sessions and transactions are preserved by tests.
