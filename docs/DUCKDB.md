# DuckDB files

DuckDB support is a source-build feature after `v0.1.0-preview.1`. That downloadable preview contains SQLite, PostgreSQL, MySQL and MariaDB; it does not include this driver.

Choose **New connection → DuckDB**. Browse to an existing `.duckdb` file, or use **+** to choose a new file. A file is created only when connecting with the create option. **Test connection** uses an isolated read-only session, never creates the file and does not save connection metadata. DuckDB files need no password, TLS or SSH configuration.

Connections to the same canonical file share a native database instance while keeping separate sessions and transactions. DuckDB cannot change an open instance’s read-only mode. Disconnect its read-write connections before using Test connection, which opens a read-only probe. A rejected test preserves the existing session and transaction. Native file locks can also prevent opening a file used by another process.

The driver embeds DuckDB 1.5.6 through the pinned Rust wrapper. It exposes local schemas, tables/views, columns, primary keys, constraints, index definitions and table/view DDL. Table browsing reuses database filters, sorting and pagination. Run SQL in tabs, use native cancellation and export retained results as CSV, typed JSON/JSONL, SQL INSERT or Markdown. SQL binary exports use DuckDB's `from_hex`, rather than SQLite's binary literal syntax.

```sql
CREATE SCHEMA analytics;
CREATE TABLE analytics.sales (
    id BIGINT PRIMARY KEY,
    amount DECIMAL(38, 18)
);
INSERT INTO analytics.sales VALUES (1, 123.456789012345678901);
SELECT * FROM analytics.sales;
```

Multiple original SQL statements produce separate result sets. DML without `RETURNING` reports the native affected count. Queries preserve NULL, booleans, binary bytes, exact decimal text and top-level signed/unsigned 128-bit integers. Other supported scalar and container values use native Arrow text. BIGNUM, BIT, TIME WITH TIME ZONE and nested 128-bit integers currently require an explicit `CAST(... AS VARCHAR)`; unsupported decoding returns an error instead of altering values. This slice does not provide a native JSON/container editor.

Use explicit `BEGIN`, `COMMIT` and `ROLLBACK` for SQL changes. The transaction indicator probes the native transaction ID, including aborted transactions requiring rollback. Confirmed reconnect discards uncommitted changes and opens a fresh session without replaying SQL. Reviewed grid editing, CSV/JSON/SQL file import and relationship diagrams remain unavailable through the capability flags; SQL writes remain available in the editor.

Choose **Explain** for the native physical JSON plan, or **Analyze** for a confirmed runtime plan. The existing collapsible tree, raw output, copy and result exports retain DuckDB operator names and fields. Estimated plans report cardinalities; runtime `latency` and `operator_timing` use seconds. Parallel operator times do not add up to elapsed time. Analyze executes writes and does not roll them back automatically. It is disabled on read-only connections; the native engine can also reject estimated write plans on read-only files. Plans use the normal timeout/cancellation pipeline. See [execution plans](EXPLAIN.md).

DuckDB results use lazy native chunk fetching with fallible conversion, 256-row / 256 KiB transport batches and the shared disk result spool. Cancellation interrupts native execution and releases blocked sends. The interruption watcher finishes before another query can use the session. A row is limited to 8 MiB; the editor and result-spool limits still apply. DuckDB may materialize some SQL operators internally. Each open database instance starts with two native worker threads and a 256 MB DuckDB buffer-manager limit; this is not a total process-memory guarantee, and desktop footprint remains to be measured.

Automatic extension installation/loading and external file/network access through DuckDB SQL are disabled. This includes `read_csv`, `read_parquet`, `COPY` to external files, external attachments and network extensions. The app's own native result exports remain available. Broader analytical file workflows need explicit permission controls and remain on the roadmap.

Run the real embedded-file checks with:

```sh
cargo test --locked --workspace
```

The workspace run includes the real DuckDB driver and core contracts. `./scripts/check.sh` adds the full local checks and runs configured real-server contracts from the same compiled test executables. Selecting individual packages or test targets can produce another native build because Cargo resolves different host dependency features. The first source build compiles DuckDB C++; it takes longer and requires more disk space than rebuilding the existing drivers. See [compatibility](COMPATIBILITY.md) and [validation evidence](VALIDATION.md) for the verified platform scope.
