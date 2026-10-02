# Security and privacy

Credentials use macOS Keychain, Windows Credential Manager or Linux Secret Service. If secure storage fails, saving fails; users can choose a session-only password. Connection URLs are parsed and stripped of embedded passwords before local persistence. Only supported URL parameters are accepted.

PostgreSQL and MySQL/MariaDB use certificate and hostname verification with OS trust roots. TLS is required by default, without plaintext fallback. `sslmode=disable` (PostgreSQL) or `tls=disabled` (MySQL/MariaDB) is an explicit opt-out for trusted local development. Optional PEM/DER CA certificates are loaded and validated by Rust from a saved file path; server certificate and hostname checks remain enabled. A custom CA requires TLS without plaintext fallback. Explicit PostgreSQL `sslmode=prefer` can fall back to plaintext when no custom CA is set. Client certificate authentication and SSH remain pending. See [TLS configuration](docs/TLS.md).

The frontend has no shell or general filesystem permissions. Database paths are explicit user inputs. Export paths come from Rust native dialogs. Export writes a temporary file and replaces the destination only after success. The production CSP restricts scripts and connections to packaged UI and IPC; there is no remote content, CDN, telemetry or schema upload.

Read-only mode and destructive SQL confirmations reduce accidents. They do not replace database roles, permissions, backups or DBA review. SQL parsing rejects unrecognized syntax rather than silently bypassing validation. Production connections are labeled. Actual manual transaction state is displayed; further safeguards remain on the roadmap.

Executable MySQL/MariaDB comments (`/*!...*/`, `/*M!...*/`, including version guards) are rejected before query submission because their meaning depends on the server and version. Write the SQL explicitly so read-only and destructive-query validation can inspect it. Ordinary comments, optimizer hints and SQL strings remain supported.

Runtime ANALYZE always requires confirmation, including SELECT plans and PostgreSQL's parenthesized ANALYZE option. It can execute writes and side effects; Klyndb does not automatically roll back the statement. Read-only connections disable runtime analysis. Estimated plans remain non-executing requests; server read-only restrictions also apply.

Local state includes SQL history, editor text and saved queries. These may contain sensitive SQL literals even though connection passwords are excluded. Avoid putting credentials in queries. Clear history when appropriate. Result spools are private temporary directories removed when released or on normal shutdown; unexpected termination may leave files for OS temporary-directory cleanup.

Logs record engine type, timings and failure booleans; they omit submitted SQL and credentials. Dependency checks use `cargo audit` and `npm audit`. Advisory exceptions, if any, must be documented rather than silently ignored.

Report vulnerabilities privately through GitHub's security reporting for OthmaneBlial/klyndb. Do not publish credentials or exploitable database dumps in public issues.

## Current transitive advisories

The 2026-10-02 cargo audit returned zero vulnerability entries and two warnings: RUSTSEC-2024-0370 (`proc-macro-error`, unmaintained) and RUSTSEC-2024-0429 (`glib` 0.18 iterator unsoundness). Both enter through Tauri's Linux GTK dependency graph. Klyndb does not call the affected glib iterator API; this does not prove the entire upstream stack unaffected. No advisories are suppressed. Track upstream GTK/Tauri upgrades and reassess before Linux releases.

Staged MySQL/MariaDB editing checks the actual InnoDB table engine and binds all values. Conversion warnings abort the batch. Nontransactional trigger effects cannot be rolled back by the server; unconfirmed rollback/interruption closes the connection and asks the user to verify data before retrying. COMMIT is never deliberately interrupted after submission.

CSV imports use private immutable snapshots, opaque IPC IDs and bound insert values. Native file selection, Rust production/read-only checks, bounded retention and whole-file deadlines protect the write path. Disconnect and Quit wait for cancellation cleanup. Server transaction aborts and nontransactional effects can prevent savepoint restoration; unconfirmed rollback closes the session and requires verification. The [import contract](docs/IMPORTS.md) documents limits and nontransactional effects.

PostgreSQL cancellation also covers blocked result sends and consumer loss. Completed responses are drained without sending a late cancellation packet. If an issued cancellation cannot be synchronized with the current query, the session is closed and the error asks users to verify writes. A read-only cleanup failure also closes the session. SQLite cancellation under lock contention can wait for the five-second native busy timeout.
