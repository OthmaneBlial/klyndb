# CSV import engine

The Rust import engine is implemented; desktop file selection, mapping, confirmation and progress UI are still pending. CSV imports are not yet available from the application. JSON and SQL file imports remain on the roadmap.

`klyndb-import` copies an explicitly selected regular file into a private temporary snapshot. Preview and execution read those same bytes, even if the original file changes. The source is read incrementally; file contents are not transferred to React. Snapshot size is limited to 512 MiB, decoded records to 8 MiB and records to 1,000 fields. A desktop registry must bound the number and total size of retained snapshots before exposing this engine through IPC.

CSV requires UTF-8 and a header row. An optional UTF-8 BOM, CRLF, embedded quoted newlines and doubled quotes are supported. Malformed quotation, inconsistent field counts and invalid UTF-8 fail explicitly. Comma, semicolon, tab and pipe separators are supported. The preview contains up to five rows with clipped values. Headers are limited to 256 bytes each and 64 KiB total.

Mapping is positional: each source field maps to one existing, non-generated destination column or is ignored. A destination can appear only once. Omitted destination columns use database defaults. Mapped values explicitly select text, number, boolean, hexadecimal binary or JSON. Numeric strings and JSON number tokens retain precision in Rust; the destination database still applies its own type and coercion rules. Optional whitespace trimming, an exact NULL token and empty-as-NULL are explicit choices; empty fields otherwise remain empty text.

The producer sends bounded batches (up to 256 rows, normally 256 KiB) to the driver. A single larger row can use the existing 8 MiB mutation bound. A distinct completion message is sent only after successful EOF validation. A dropped producer never authorizes commit. The parsed-row counter is progress, not a count of committed rows.

SQLite, PostgreSQL and MySQL/MariaDB InnoDB drivers hold their session lock for the entire stream, including waits for the parser. Another tab cannot commit between batches. An import owns a transaction when needed, or uses a savepoint inside the user's existing transaction. Success in a manual transaction remains pending for the user to commit or roll back. A parser error, database error or cancellation rolls back all import batches while preserving earlier caller-owned work.

Read-only connections, generated columns and non-insert mutations are rejected. MySQL/MariaDB additionally reject non-InnoDB destination tables and conversion warnings. Nontransactional trigger effects can survive rollback; detected incomplete rollback closes the session and requires data verification before retrying. Database external effects are not guaranteed reversible.

Cancellation interrupts native work and parser waits. Final COMMIT/RELEASE is not deliberately interrupted after submission. PostgreSQL requires a synchronized cancellation response before session reuse; uncertain termination closes the connection. SQLite lock contention can still wait for its five-second busy timeout. Per-file deadlines, production confirmation and disconnect/job ownership must be wired into the core before desktop imports are enabled.

`crates/core/tests/import.rs` exercises the real bounded CSV-to-driver flow. With disposable server URLs it covers SQLite, PostgreSQL and either MySQL or MariaDB: 1,201 quoted/multiline records, late parser failure, duplicate keys, producer loss, cancellation, manual transactions, concurrent COMMIT exclusion, read-only guards and incomplete nontransactional trigger rollback. `klyndb-import` also has a runnable parser/mapping/snapshot contract.
