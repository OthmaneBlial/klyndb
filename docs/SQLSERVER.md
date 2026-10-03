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

Query deadlines follow the configured Klyndb timeout, without an additional SDK 30-second timer. Metadata has a separate 30-second driver bound; the inspector has its shared shorter deadline.

The configured result limit applies per native result set. Reaching it stops the remainder of the original batch; later writes may already have executed. Cancellation sends native TDS Attention and drains its acknowledgement before reuse. If synchronization cannot be confirmed within three seconds, the connection closes and asks you to reconnect and verify writes. Cancellation never promises rollback.

## Tables and transactions

The selected database exposes schema-qualified tables/views, columns, nullability, primary-key/generated flags and indexes. Structure also shows ordered foreign-key column mappings, primary/unique/foreign/check/default constraint names and types, native CHECK/default definitions, and user trigger bodies with enabled/disabled and AFTER/INSTEAD OF state. Definitions follow server permissions and encrypted/CLR definitions can be unavailable. View definitions use the native catalog. Base-table CREATE scripts, complete key constraint scripts, statistics, additional database/routine/role browsing and schema editing remain pending. Browsing uses native TOP and ORDER BY/OFFSET/FETCH pages; the shared filter/sort controls are enabled.

Execute explicit BEGIN TRANSACTION, COMMIT or ROLLBACK in SQL. The transaction indicator uses native XACT_STATE, including failed transactions. Confirmed reconnect drops the original session, rolling back its open transaction and removing temporary tables; retained result exports stay available.

Read-only mode blocks non-read-only SQL in Klyndb. SQL Server has no per-session native read-only switch in this implementation: use a database principal with restricted server permissions for a server-enforced boundary. CSV/JSON row imports are enabled; Relationship diagrams are also enabled on read-only connections; SQL file imports and structured execution plans remain disabled for this driver.

## Reviewed table editing

Open a disk-based base table, stage inserts/updates/deletions, review and apply through the existing grid. Updates/deletes need a non-NULL primary key. Views, memory-optimized tables and tables with enabled INSTEAD OF triggers are excluded. Identity, computed, rowversion and generated columns are protected; omitted insert columns use native defaults.

Values use bound parameters and native destination-type casts. Numeric conversion checks compare exact decimal tokens without f64 rounding, and supported text/binary casts are checked for truncation or encoding loss. Fixed char/binary padding is allowed. Temporal, XML and UUID values follow native conversion rules. Writing user-defined alias/CLR columns is unsupported by this cast-based writer; [SQL Server does not allow alias types as CAST targets](https://learn.microsoft.com/en-us/sql/t-sql/functions/cast-and-convert-transact-sql?view=sql-server-ver17). Unsupported native codecs require SQL casts outside grid editing. Destination precision and length appear in Structure.

The complete original row is reread under a lock and compared in Rust before update/delete, including differences hidden by case-insensitive collation. A stale row, conversion error or constraint failure aborts the batch. Native `OUTPUT 1 INTO` counts direct changes without adding AFTER-trigger row counts. Arbitrary query affected counts remain unavailable.

A successful batch commits only when it started with no transaction and IMPLICIT_TRANSACTIONS OFF. Existing explicit transactions and implicit-transaction mode leave edits pending for COMMIT/ROLLBACK. Savepoints preserve earlier caller work on recoverable failures. Session transaction/SET commands use native batches; parameterized value requests use RPC. Original IMPLICIT_TRANSACTIONS, ANSI_WARNINGS and ARITHABORT settings are restored.

Batches are limited to 1,000 changes / 8 MiB and one 60-second deadline. Attention acknowledgement and rollback must be confirmed before reuse; an uncertain interruption, rollback or commit closes the connection and requires write/transaction verification before retrying. A failed/uncommittable transaction must be rolled back first. Server triggers and external side effects retain their native transaction limits.

The current writer holds a table-wide exclusive lock through the transaction to protect metadata and reread values. In a manual transaction this lock remains until COMMIT/ROLLBACK and can block other sessions. Narrower locks are a follow-up, rather than an unverified concurrency guarantee.

## CSV and JSON row imports

Choose **Import data** on an editable table and select CSV, JSON object array or Klyndb JSON export. The shared native picker, immutable snapshot, preview, explicit field/type mapping and production confirmation flow is used. Identity/generated columns cannot be mapped; omitted columns use defaults. [The import guide](IMPORTS.md) describes formats and file/record bounds.

The native writer holds the session lock and one transaction/savepoint across every bounded batch and parser wait. An explicit completion message after valid EOF is required before success; producer loss, late malformed records, conversion errors and constraint failures roll back earlier imported rows. Existing explicit/implicit transactions leave successful imports pending for COMMIT/ROLLBACK, preserving earlier caller work on recoverable errors. Values use the same bound casts and exact conversion checks as reviewed grid edits.

The configured query timeout supplies the whole-file 1–3,600-second job deadline; the native driver also caps a stream at one hour. Cancellation interrupts parser waits or native requests, drains Attention and confirms rollback before reuse. Final COMMIT is not deliberately interrupted, and a late cancel can arrive after commit. Unconfirmed cleanup or commit closes the connection and requires verification before retrying. Native trigger/external-effect limits still apply.

The current implementation makes per-row conversion/write requests and holds the same table-wide exclusive lock until the transaction ends. Large-file throughput remains unmeasured. Views, memory-optimized tables, enabled INSTEAD OF triggers and unsupported alias/CLR destination casts retain the editing restrictions. SQL file imports remain disabled, including GO-based dumps.

## Relationship diagrams

Choose **Relationships** above the connected table list or **Open relationship diagram** in the command palette. Select a focused group of tables and load their columns, primary keys and native foreign-key relationships. Composite column pairs keep SQL Server constraint order; cross-schema targets and self-references are retained. Targets outside the selection remain listed so you can add them.

The existing diagram view provides manual/grid layout, keyboard positioning, pan/zoom/Fit, saved local positions and native SVG export. Read-only connections can inspect the same graph without enabling writes. Metadata visibility follows server permissions. The shared limits are 50 tables, 2,000 columns, 4,000 relationship column pairs, 4 MiB of metadata and a 30-second load deadline. See [diagram controls and limits](DIAGRAMS.md).

The graph reuses the existing bounded, session-serialized catalog requests. Loading reads metadata rather than table contents; it is not a frozen schema snapshot. If a metadata request times out while reading a response, its owned native client closes and reconnect is required. Native SQL Server desktop diagram acceptance remains pending.

## Evidence and remaining validation

The real native driver contract uses Microsoft SQL Server 2022 CU27, `16.0.4295.3`, Developer Edition from the official container image. It checks exact cells, DECLARE/multiple/empty result sets, native writes/catalog/browsing/transactions, reviewed edits and conflicts, cancellation and reuse, consumer loss, row limits and TLS encryption/CA/hostname rejection. The fixture runs under Rosetta on Apple Silicon; [Microsoft supports these Linux containers on x86-64 hosts](https://learn.microsoft.com/en-us/sql/linux/install-upgrade/quickstart-install-docker?view=sql-server-linux-ver15), so this is protocol test evidence, not a supported ARM production deployment claim.

The real core contract also passes for isolated connection testing, saved sessions, exact disk-spool cells, production editing/import confirmation, immutable CSV and standard/typed JSON imports, late-error rollback, manual-transaction export roundtrip, one-second slow-trigger import rollback/reuse, multi-result CSV/SQL export, native SQL-export roundtrip and confirmed reconnect that rolls back the original transaction, removes temporary tables and retains completed results. Native desktop acceptance and Windows/Linux workflows remain pending and are tracked separately in [VALIDATION.md](VALIDATION.md). This source driver is not a new published binary release.
