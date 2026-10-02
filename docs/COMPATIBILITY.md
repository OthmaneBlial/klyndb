# Compatibility and evidence

| Engine | Connect/query | Schema | Cancel | Edit | TLS | Local verification |
| --- | --- | --- | --- | --- | --- | --- |
| SQLite | Implemented | Tables/views, columns, indexes, FKs, DDL | VM progress handler | Bound insert/update/delete batches | Not applicable | Real-file unit and engine workflow tests |
| PostgreSQL | Implemented, text protocol cells | Tables/views, columns, indexes, FK definitions | Server cancel | Bound insert/update/delete batches | OS trust, verify by default | PostgreSQL 16 real-server contract (local and CI) |
| MySQL/MariaDB | Implemented, streamed text protocol with typed cells | Tables/views, columns, indexes, FKs, DDL | Separate-session KILL QUERY with protocol drain | Bound InnoDB insert/update/delete batches | Required, OS trust and hostname verification by default | Real MySQL 8.4.11 and MariaDB 13.0.2 contracts; native macOS MySQL workflow |
| SQL Server | Pending | Pending | Pending | Pending | Pending | None |
| DuckDB/ClickHouse | Pending | Pending | Pending | Pending | Pending | None |
| MongoDB/Redis | Pending | Pending | Pending | Pending | Pending | None |
| Other requested engines | Pending | Pending | Pending | Pending | Pending | None |

PostgreSQL simple-protocol results preserve server text and NULL; binary/date/JSON/array values currently appear as text unless inspected as JSON. Typed codecs are a follow-up. No claim of CockroachDB or Redshift compatibility follows merely from PostgreSQL wire compatibility.

MySQL/MariaDB preserves integer and decimal text exactly as tagged numbers, distinguishes binary columns and NULL, and keeps one dedicated user session for manual transactions. Non-UTF-8 result text is preserved as binary bytes; automatic legacy-charset decoding remains pending. MySQL JSON protocol values use the typed JSON viewer; MariaDB JSON aliases appear as text. Cancellation falls back to closing the session if interruption cannot be confirmed within three seconds; reconnect is then required. Custom certificates and a successful trusted-TLS server workflow remain pending.

Result sorting and filtering are local to the current 500-row page. Export includes only the rows retained by the configured row limit. Native exports stream one row at a time from the result spool; the 8 MiB UI page bound does not limit the exported file. JSON/JSONL exports use `{columns, values}` with tagged cells to preserve duplicate names, NULL and large integers. SQL INSERT exports use standard quoted identifiers and SQLite-style binary literals; cross-dialect binary export remains pending.

Table editing is enabled only for the built-in table-opening SELECT and matching full column order. Ad-hoc SQL results remain read-only. Updates/deletes require a non-NULL primary key and compare every original value; a stale row cancels the entire batch. Views and generated columns cannot be directly changed. Batches are bounded to 1,000 changes / 8 MiB. Inserts can omit columns for database defaults. Savepoints preserve an existing manual transaction; after editing its changes require an explicit COMMIT or ROLLBACK. PostgreSQL compares the server's text representation; changing its formatting settings can cause a conservative conflict.

MySQL/MariaDB staged batches require an InnoDB base table and UTF-8 session (`SET NAMES utf8mb4` after changing session encodings). Server metadata is checked while holding the table metadata lock. Bound unsigned integers and decimal text preserve exact values; binary comparisons detect changes hidden by case-insensitive/trailing-space collation. Value-conversion warnings abort the batch. Manual transactions, including `autocommit=0`, remain uncommitted. The 60-second editing deadline interrupts and drains the protocol before rollback; COMMIT acknowledgements are drained without cancelling the commit. An unconfirmed interruption/rollback closes the connection and requires data verification before retrying. MySQL UPDATE affected counts report matched rows, including unchanged values.

Transaction guarantees follow the server: nontransactional trigger writes and external side effects cannot be undone by a savepoint. Rollback warnings are surfaced rather than reported as atomic success. Server formatting/encoding changes may produce conservative conflicts; refresh before editing.
