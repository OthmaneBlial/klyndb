# SQL Server in source builds

SQL Server is available in source builds after Preview 1. The existing downloadable `v0.1.0-preview.1` still contains SQLite, PostgreSQL, MySQL and MariaDB only.

Choose **SQL Server** in the connection dialog, enter a native TCP URL and use the separate password field:

```text
mssql://user@db.example.com:1433/database
```

SQL authentication is supported. TLS certificate and hostname verification are enabled by default. The CA picker accepts a PEM bundle or DER certificate; the connection timeout is 1–300 seconds, default 10. Explicit `tls=disabled` is available for a trusted local fixture. Named-instance discovery, Windows/integrated authentication, Azure token authentication, strict TDS 8.0 and TLS client identities remain pending. The shared single-bastion SSH adapter is wired with port 1433 and preserves the original TLS hostname; a real SQL Server SSH contract is still pending.

## Queries and results

The original selection/file is sent as **one T-SQL batch**, preserving local variables between statements:

```sql
DECLARE @maximum bigint = 9223372036854775807;
SELECT @maximum AS exact_integer;
SELECT CAST('-0.000000000000000001' AS decimal(38,18)) AS exact_decimal;
```

Use Execute all or select the complete batch for variables used across statements. Execute current statement submits just that statement. `GO` is a client batch directive and is not supported. SQL must pass the shared T-SQL validator; some vendor DDL, including computed-column declarations, remains unsupported by its grammar. Unsupported SQL fails before submission.

Native SELECT/OUTPUT result sets share the bounded Rust result spool, virtualized grid and CSV/typed JSON/JSONL/SQL/Markdown exporters. Empty SELECT sets are retained. The native stream does not expose DONE affected counts, so the UI says **Affected-row count unavailable**; it does not invent a zero. DML without a rowset produces an empty completion, and DML between SELECT sets does not create a separate artificial rowset.

BIGINT and DECIMAL/NUMERIC values remain exact text tokens in Rust, including precision 38. NULL, Unicode text and binary values remain distinct. The client currently decodes MONEY/SMALLMONEY through floating point, so Klyndb rejects those columns: cast them to DECIMAL in the SELECT. Native unsupported representations need an explicit text cast. SQL exports use bracket identifiers, Unicode `N'…'` strings, `0x…` binary and numeric bit literals.

The configured result limit applies per native result set. Reaching it stops the remainder of the original batch; later writes may already have executed. Cancellation sends native TDS Attention and drains its acknowledgement before reuse. If synchronization cannot be confirmed within three seconds, the connection closes and asks you to reconnect and verify writes. Cancellation never promises rollback.

## Tables and transactions

The selected database exposes schema-qualified tables/views, columns, nullability, primary-key/generated flags and indexes. Structure also shows ordered foreign-key column mappings, primary/unique/foreign/check/default constraint names and types, native CHECK/default definitions, and user trigger bodies with enabled/disabled and AFTER/INSTEAD OF state. Definitions follow server permissions and encrypted/CLR definitions can be unavailable. View definitions use the native catalog. Base-table CREATE scripts, complete key constraint scripts, statistics, additional database/routine/role browsing and schema editing remain pending. Browsing uses native TOP and ORDER BY/OFFSET/FETCH pages; the shared filter/sort controls are enabled.

Execute explicit BEGIN TRANSACTION, COMMIT or ROLLBACK in SQL. The transaction indicator uses native XACT_STATE, including failed transactions. Confirmed reconnect drops the original session, rolling back its open transaction and removing temporary tables; retained result exports stay available.

Read-only mode blocks non-read-only SQL in Klyndb. SQL Server has no per-session native read-only switch in this implementation: use a database principal with restricted server permissions for a server-enforced boundary. Grid editing, file imports, structured execution plans and diagrams remain disabled for this driver.

## Evidence and remaining validation

The real native driver contract uses Microsoft SQL Server 2022 CU27, `16.0.4295.3`, Developer Edition from the official container image. It checks exact cells, DECLARE/multiple/empty result sets, native writes/catalog/browsing/transactions, cancellation and reuse, consumer loss, row limits and TLS encryption/CA/hostname rejection. The fixture runs under Rosetta on Apple Silicon; [Microsoft supports these Linux containers on x86-64 hosts](https://learn.microsoft.com/en-us/sql/linux/install-upgrade/quickstart-install-docker?view=sql-server-linux-ver15), so this is protocol test evidence, not a supported ARM production deployment claim.

The real core contract also passes for isolated connection testing, saved sessions, exact disk-spool cells, multi-result CSV/SQL export, native SQL-export roundtrip and confirmed reconnect that rolls back the original transaction, removes temporary tables and retains completed results. Native desktop acceptance and Windows/Linux workflows remain pending and are tracked separately in [VALIDATION.md](VALIDATION.md). This source driver is not a new published binary release.
