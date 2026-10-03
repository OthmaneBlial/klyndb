# ClickHouse native connections

ClickHouse is a source-build feature after `v0.1.0-preview.1`. The downloadable Preview 1 contains SQLite, PostgreSQL, MySQL and MariaDB. Build the current source to use ClickHouse or DuckDB.

Choose **New connection → ClickHouse** and enter the native TCP URL. HTTP ports such as 8123 are not supported by this driver.

```text
clickhouse://default@localhost:9000/default?tls=disabled
clickhouse://alice@database.example.com:9440/analytics
```

Verified TLS is the default, with port 9440 when no port is specified. Explicit `tls=disabled` selects plaintext and defaults to port 9000; use it only for a trusted local server. The separate password field and OS keychain use the existing connection flow. PEM/DER custom CAs, PKCS#12 client identities and the pinned single-bastion SSH transport share the existing Rust security controls. Actual custom-CA, hostname rejection and server-enforced read-only contracts pass; ClickHouse client-certificate and SSH integration checks remain pending.

The connection deadline is 1–300 seconds, default 10, covering TCP, TLS and native handshake. **Test connection** opens an isolated read-only session without saving metadata or changing the existing session. Reconnect opens a fresh session without replaying SQL; completed result exports remain available.

Browse the selected database's tables/views, columns, data-skipping indexes and native DDL. MergeTree primary/sorting keys are not relational uniqueness constraints, so the driver does not mark those columns as unique editable keys. Table browsing supports filtering, sorting and database pages. Contains filters use native `position` rather than an unsupported LIKE ESCAPE clause.

```sql
CREATE TABLE events (
    id UInt64,
    label Nullable(String)
) ENGINE = MergeTree ORDER BY id;
INSERT INTO events VALUES (1, 'hello'), (2, NULL);
SELECT id, label FROM events ORDER BY id;
```

SQL tabs execute original statements and return separate result sets. Top-level signed/unsigned integers through 256 bits, Decimal32/64/128/256, UTF-8 strings, NULL, UUIDs, supported dates/timestamps and IP addresses are mapped without converting exact numbers into JavaScript floats. Non-UTF-8 String bytes are retained as binary hex; a String containing valid UTF-8 appears as text. Native Bool aliases arrive as UInt8 numbers. Nested/container types and unsupported native codecs currently require `CAST(value AS String)`. Use distinct column aliases: duplicate native result names are rejected instead of overwriting data. A native codec failure can close the session; reconnect with distinct aliases or String casts.

Results use 256-row / 256 KiB output batches and the existing bounded disk spool, with an 8 MiB row limit. Native blocks are requested at 256 rows / 256 KiB where permissions allow; restricted profiles keep their server block settings. These sizes do not guarantee total application RSS or bound every server operator. Exports support CSV, typed JSON/JSONL, SQL INSERT and Markdown. SQL INSERT exports use `unhex` for ClickHouse strings and binary bytes; the real-server core contract checks a scalar SQL export roundtrip.

Cancel, timeout, row-limit interruption and consumer loss target a UUID-prefixed statement belonging to the authenticated user, using a separate native control connection. The driver keeps draining the original stream before reusing the session. If interruption cannot be confirmed within three seconds, it closes the session and asks you to verify any writes before retrying. Cancellation does not roll back completed writes or cancel asynchronous mutations already launched by SQL. See [ClickHouse's KILL semantics](https://clickhouse.com/docs/reference/statements/kill).

Choose **Explain** for a single SELECT statement or selection, including native WITH/UNION queries. The driver requests native JSON operators and index details through `EXPLAIN PLAN json=1, indexes=1`, retaining original SQL and reusing the collapsible tree, raw output, copy and result exports. Index conditions/parts/granules are shown when the server reports them; no runtime timings or cost estimates are invented. Read-only native profiles support the same plans. Planning can contact table-function sources to infer their schema, so normal permissions, Cancel and job deadlines still apply. Unconfirmed interruption closes the session with a warning; reconnect before retrying. The verified ClickHouse 26.3.39.7 server rejects ANALYZE, and runtime profiling remains disabled in this driver. Native macOS source debug estimated tree/raw interaction passes; packaged and Windows/Linux plan interaction remain pending. See [execution plans](EXPLAIN.md).

ClickHouse transactions, reviewed grid editing, file imports and diagrams remain disabled in this slice. Affected-row counts are not exposed by this native client; the results and messages display “Affected-row count unavailable” rather than a measured number of changed rows. Native macOS source debug connection/query/catalog/DDL, exact UInt64/UInt256/decimal values, estimated tree/raw plans, CSV export and Cancel with same-session reuse pass. The UI check uses an owned loopback TCP fixture with TLS disabled; it does not establish native TLS acceptance. Multi-database navigation, native TLS checks, platform packages and performance measurements remain on the roadmap. See [native workflow evidence](VALIDATION.md#native-sql-server-and-clickhouse-source-workflows--2026-10-03).

Real-server contracts run against a disposable native server:

```sh
export KLYNDB_TEST_CLICKHOUSE_URL='clickhouse://default@127.0.0.1:19000/default?tls=disabled'
# Optional: a native local server can reach the loopback HTTP schema-delay fixture.
export KLYNDB_TEST_CLICKHOUSE_DELAY_HOST='127.0.0.1'
./scripts/check.sh
```

For the TLS contract, also configure `KLYNDB_TEST_TLS_CLICKHOUSE_URL`, `KLYNDB_TEST_TLS_CERT_DIR` with the documented test certificates, and a disposable `readonly_fixture` user whose server profile enforces `readonly=1`. Keep credentials out of saved scripts and source. The client uses pinned [klickhouse](https://github.com/Protryon/klickhouse) with the small, documented safety patches in [the retained client source](../third_party/klickhouse-0.15.3/KLYNDB_PATCH.md). Original licenses and bundled LZ4 notices are included in the package inventory.

The exact optimized macOS Preview 2 candidate now passes local TCP connection/catalog, exact UInt64/UInt256/decimal/Unicode/binary values, Structure DDL, estimated tree/raw index details, independently checked native CSV export and Cancel with same-session reuse. TLS is explicitly disabled only on the owned loopback fixture; native TLS/client identity/SSH and other platforms remain pending. This candidate is unpublished; [validation evidence](VALIDATION.md#preview-2-exact-package-sql-server-clickhouse-mongodb-and-redis-acceptance--2026-10-03) does not change the Preview 1 download.
