# SQL Server in source builds

SQL Server is available in [Preview 2 for macOS Apple Silicon](RELEASES.md#preview-2--macos-apple-silicon) and current source builds. Preview 1 contains only SQLite, PostgreSQL, MySQL and MariaDB.

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

Read-only mode blocks non-read-only SQL in Klyndb. SQL Server has no per-session native read-only switch in this implementation: use a database principal with restricted server permissions for a server-enforced boundary. CSV/JSON row imports and relationship diagrams are enabled. Estimated plans are available on read-only connections; runtime Analyze requires a writable connection and explicit confirmation. Reviewed SQL file imports are enabled on writable connections.

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

The current implementation makes per-row conversion/write requests and holds the same table-wide exclusive lock until the transaction ends. Large-file throughput remains unmeasured. Views, memory-optimized tables, enabled INSTEAD OF triggers and unsupported alias/CLR destination casts retain the editing restrictions. SQL files use the separate reviewed batch workflow below.

## SQL files and native batches

Choose **Import SQL** on a connected SQL tab, select a UTF-8 file through the native picker, review the immutable whole-file preflight and confirm execution. The dialog counts **batches**, rather than individual statements. Standalone case-insensitive `GO` lines separate native requests; semicolons within a batch preserve DECLARE variables and original SQL. Quoted strings/identifiers and nested comments keep embedded GO text. The editor uses the same framing when executing a selected batch or Execute all. [Microsoft describes GO and variable scope](https://learn.microsoft.com/en-us/sql/t-sql/language-elements/sql-server-utilities-statements-go?view=sql-server-ver17).

Files are limited to 512 MiB, with a 4 MiB native batch bound. Split independent units with GO. SQL INSERT exports add GO after each insertion and remain executable through the editor or import workflow. The shared parser checks every batch before execution; client commands such as `:r`, SQLCMD substitutions, GO repeat counts/GO semicolons and vendor syntax outside the parser remain unsupported. Imports require QUOTED_IDENTIFIER ON and SHOWPLAN disabled; assignments that turn QUOTED_IDENTIFIER off are rejected during preflight. Explicit ON assignments are allowed. The native session property is checked before every batch, without opening an implicit table transaction.

One session lock spans the whole file, including producer waits. SELECT results are fully drained and discarded, without a retained-row cap stopping later SQL. Progress increments only after a native batch finishes. The script owns its transactions: no automatic whole-file transaction or rollback is added. Earlier batches and effects within a failing batch can remain committed; SQL Server can continue statements within that batch after some native errors. Verify data and use COMMIT/ROLLBACK as appropriate before retrying. Cancellation/deadlines stop subsequent batches and drain native Attention before reuse. An uncertain interruption closes the connection; a dropped execution future also closes its owned client.

Native SQL Server desktop file-picker/review/cancel interaction remains pending. See [shared import controls](IMPORTS.md).

## Relationship diagrams

Choose **Relationships** above the connected table list or **Open relationship diagram** in the command palette. Select a focused group of tables and load their columns, primary keys and native foreign-key relationships. Composite column pairs keep SQL Server constraint order; cross-schema targets and self-references are retained. Targets outside the selection remain listed so you can add them.

The existing diagram view provides manual/grid layout, keyboard positioning, pan/zoom/Fit, saved local positions and native SVG export. Read-only connections can inspect the same graph without enabling writes. Metadata visibility follows server permissions. The shared limits are 50 tables, 2,000 columns, 4,000 relationship column pairs, 4 MiB of metadata and a 30-second load deadline. See [diagram controls and limits](DIAGRAMS.md).

The graph reuses the existing bounded, session-serialized catalog requests. Loading reads metadata rather than table contents; it is not a frozen schema snapshot. If a metadata request times out while reading a response, its owned native client closes and reconnect is required. Native SQL Server desktop diagram acceptance remains pending.

## Execution plans

Select one SELECT, INSERT, UPDATE or DELETE statement and choose **Explain** for a native estimated plan, or **Analyze** for confirmed runtime execution. The shared view shows an operator tree, all original fields, native warnings and raw tabular output. Estimated SHOWPLAN_ALL does not execute the original statement. STATISTICS PROFILE executes it and reports actual Rows and Executes alongside estimated rows, CPU/I/O costs and subtree costs. Those costs remain optimizer estimates, not wall-clock timings; per-operator runtime timings are not provided. [Microsoft documents the estimate format](https://learn.microsoft.com/en-us/sql/t-sql/statements/set-showplan-all-transact-sql?view=sql-server-ver17) and [runtime profile fields](https://learn.microsoft.com/en-us/sql/t-sql/statements/set-statistics-profile-transact-sql?view=sql-server-ver17).

The session lock spans separate native setup/query/cleanup batches. Settings must be restored before reuse; cancellation drains Attention first, and uncertain cleanup closes the connection and asks you to verify writes/transactions. Analyze does not roll back writes. A caller-owned transaction remains yours to COMMIT/ROLLBACK. Already enabled SHOWPLAN/STATISTICS PROFILE/XML settings are rejected without intentionally changing them; disable those settings before requesting a plan.

Runtime data results retain at most 5,000 rows per set, while surplus data is drained so the native profile can complete. The last native plan/profile table supplies the formatted tree; other result sets remain available in Results/export. The tree uses statement-scoped node/parent IDs, preserving all reported fields. Plans share the ordinary timeout/Cancel action and bounded conversion limits. Native statements that emit no profile, such as a constant-only SELECT, do not receive fabricated runtime metrics: the plan error states that execution completed and that writes must be verified before retrying.

The server requires SHOWPLAN permission and the relevant statement permissions for all referenced databases. Estimated DML is non-executing even on a client read-only connection, but native permissions can reject it. EXEC, session-control commands, transaction commands and DDL are excluded from this plan action. History records native setup/query/cleanup with GO batch separators to distinguish planning from plain execution; select the original statement and use Explain/Analyze to repeat it. Standalone GO is now supported in imports and editor batches; the plan action still accepts one original statement. Native macOS source debug estimated tree/raw and confirmed runtime Rows/Executes interaction pass; packaged and Windows/Linux plan checks remain pending. See [shared plan controls and limits](EXPLAIN.md).

## Evidence and remaining validation

The real native driver contract uses Microsoft SQL Server 2022 CU27, `16.0.4295.3`, Developer Edition from the official container image. It checks exact cells, DECLARE/multiple/empty result sets, native writes/catalog/browsing/transactions, reviewed edits and conflicts, cancellation and reuse, consumer loss, row limits and TLS encryption/CA/hostname rejection. The fixture runs under Rosetta on Apple Silicon; [Microsoft supports these Linux containers on x86-64 hosts](https://learn.microsoft.com/en-us/sql/linux/install-upgrade/quickstart-install-docker?view=sql-server-linux-ver15), so this is protocol test evidence, not a supported ARM production deployment claim.

The real core contract also passes for isolated connection testing, saved sessions, exact disk-spool cells, production editing/import confirmation, immutable CSV and standard/typed JSON imports, late-error rollback, manual-transaction export roundtrip, one-second slow-trigger import rollback/reuse, multi-result CSV/SQL export, native SQL-export roundtrip and confirmed reconnect that rolls back the original transaction, removes temporary tables and retains completed results. Native macOS source debug connection testing, verified TLS, multi-result batches/local variables, precise values, table Structure and reviewed Unicode updates pass. Native estimated/runtime plans, reviewed two-batch GO SQL-file import, exact CSV export and cancellation with same-session reuse also pass. The exact optimized macOS Preview 2 candidate also passes verified-TLS connection, catalog/precise cells/Structure and standard JSON field mapping/append/refresh. An independent server query verifies the imported Unicode label, exact decimal and NULL. Native diagrams, the remaining candidate workflows and Windows/Linux checks remain pending and are tracked separately in [VALIDATION.md](VALIDATION.md#native-sql-server-and-clickhouse-source-workflows--2026-10-03). This source driver is not a new published binary release.

The exact optimized macOS Preview 2 candidate additionally passes original DECLARE/multiple-result batches, estimated tree/raw and confirmed SELECT runtime Rows/Executes, real WAITFOR cancellation and same-session reuse. These specific checks are recorded separately in [VALIDATION.md](VALIDATION.md#preview-2-exact-package-sql-server-clickhouse-mongodb-and-redis-acceptance--2026-10-03); Preview 2 is now published with these exact artifacts.
