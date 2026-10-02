# Compatibility and evidence

| Engine | Connect/query | Schema | Cancel | Edit | TLS | Local verification |
| --- | --- | --- | --- | --- | --- | --- |
| SQLite | Implemented | Tables/views, columns, indexes, FKs, DDL | VM progress handler | Pending | Not applicable | Real-file unit and engine workflow tests |
| PostgreSQL | Implemented, text protocol cells | Tables/views, columns, indexes, FK definitions | Server cancel | Pending | OS trust, verify by default | Real-server contract pending |
| MySQL/MariaDB | Pending | Pending | Pending | Pending | Pending | None |
| SQL Server | Pending | Pending | Pending | Pending | Pending | None |
| DuckDB/ClickHouse | Pending | Pending | Pending | Pending | Pending | None |
| MongoDB/Redis | Pending | Pending | Pending | Pending | Pending | None |
| Other requested engines | Pending | Pending | Pending | Pending | Pending | None |

PostgreSQL simple-protocol results preserve server text and NULL; binary/date/JSON/array values currently appear as text unless inspected as JSON. Typed codecs are a follow-up. No claim of CockroachDB or Redshift compatibility follows merely from PostgreSQL wire compatibility.

Result sorting and filtering are local to the current 500-row page. Export includes only the rows retained by the configured row limit. JSON/JSONL exports use `{columns, values}` with tagged cells to preserve duplicate names, NULL and large integers. SQL INSERT exports use standard quoted identifiers and SQLite-style binary literals; cross-dialect binary export remains pending.
