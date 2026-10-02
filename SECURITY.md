# Security and privacy

Credentials use macOS Keychain, Windows Credential Manager or Linux Secret Service. If secure storage fails, saving fails; users can choose a session-only password. Connection URLs are parsed and stripped of embedded passwords before local persistence. Only supported URL parameters are accepted.

PostgreSQL and MySQL/MariaDB use certificate and hostname verification with OS trust roots. TLS is required by default; no plaintext fallback is allowed. `sslmode=disable` (PostgreSQL) or `tls=disabled` (MySQL/MariaDB) is an explicit opt-out for trusted local development. Custom certificates and SSH have not yet been implemented.

The frontend has no shell or general filesystem permissions. Database paths are explicit user inputs. Export paths come from Rust native dialogs. Export writes a temporary file and replaces the destination only after success. The production CSP restricts scripts and connections to packaged UI and IPC; there is no remote content, CDN, telemetry or schema upload.

Read-only mode and destructive SQL confirmations reduce accidents. They do not replace database roles, permissions, backups or DBA review. SQL parsing rejects unrecognized syntax rather than silently bypassing validation. Production connections are labeled. Manual transaction visibility and further safeguards remain on the roadmap.

Local state includes SQL history, editor text and saved queries. These may contain sensitive SQL literals even though connection passwords are excluded. Avoid putting credentials in queries. Clear history when appropriate. Result spools are private temporary directories removed when released or on normal shutdown; unexpected termination may leave files for OS temporary-directory cleanup.

Logs record engine type, timings and failure booleans; they omit submitted SQL and credentials. Dependency checks use `cargo audit` and `npm audit`. Advisory exceptions, if any, must be documented rather than silently ignored.

Report vulnerabilities privately through GitHub's security reporting for OthmaneBlial/klyndb. Do not publish credentials or exploitable database dumps in public issues.

## Current transitive advisories

The 2026-10-02 cargo audit returned zero vulnerability entries and two warnings: RUSTSEC-2024-0370 (`proc-macro-error`, unmaintained) and RUSTSEC-2024-0429 (`glib` 0.18 iterator unsoundness). Both enter through Tauri's Linux GTK dependency graph. Klyndb does not call the affected glib iterator API; this does not prove the entire upstream stack unaffected. No advisories are suppressed. Track upstream GTK/Tauri upgrades and reassess before Linux releases.
