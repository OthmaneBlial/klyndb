# Compatibility and evidence

| Engine | Connect/query | Schema | Cancel | Edit | TLS | Local verification |
| --- | --- | --- | --- | --- | --- | --- |
| SQLite | Implemented | Tables/views, columns, indexes, FKs, DDL | VM progress handler | Bound insert/update/delete batches | Not applicable | Real-file unit and engine workflow tests |
| PostgreSQL | Implemented, text protocol cells | Tables/views, columns, indexes, FK definitions | Server cancel | Bound insert/update/delete batches | OS trust, verify by default | PostgreSQL 16 real-server contract (local and CI) |
| MySQL/MariaDB | Pending | Pending | Pending | Pending | Pending | None |
| SQL Server | Pending | Pending | Pending | Pending | Pending | None |
| DuckDB/ClickHouse | Pending | Pending | Pending | Pending | Pending | None |
| MongoDB/Redis | Pending | Pending | Pending | Pending | Pending | None |
| Other requested engines | Pending | Pending | Pending | Pending | Pending | None |

PostgreSQL simple-protocol results preserve server text and NULL; binary/date/JSON/array values currently appear as text unless inspected as JSON. Typed codecs are a follow-up. No claim of CockroachDB or Redshift compatibility follows merely from PostgreSQL wire compatibility.

Result sorting and filtering are local to the current 500-row page. Export includes only the rows retained by the configured row limit. JSON/JSONL exports use `{columns, values}` with tagged cells to preserve duplicate names, NULL and large integers. SQL INSERT exports use standard quoted identifiers and SQLite-style binary literals; cross-dialect binary export remains pending.

Table editing is enabled only for the built-in table-opening SELECT and matching full column order. Ad-hoc SQL results remain read-only. Updates/deletes require a non-NULL primary key and compare every original value; a stale row cancels the entire batch. Views and generated columns cannot be directly changed. Batches are bounded to 1,000 changes / 8 MiB. Inserts can omit columns for database defaults. Savepoints preserve an existing manual transaction; after editing its changes require an explicit COMMIT or ROLLBACK. PostgreSQL compares the server's text representation; changing its formatting settings can cause a conservative conflict.
