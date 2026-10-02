# TLS and custom CA certificates

PostgreSQL, MySQL and MariaDB connections require verified TLS by default. The native connectors verify the server certificate chain and hostname against the OS trust store.

## Connect to a private database

1. Create or edit a server connection and enter its database URL.
2. Expand **TLS & certificates**. Keep **Verified TLS** selected.
3. Choose a public CA certificate file with the native picker. Use the hostname listed in the server certificate, rather than substituting its IP address.
4. Choose **Test connection**, then save and connect.

The file can contain a PEM certificate bundle or a DER certificate. It must be a nonempty regular file, no larger than 1 MiB, containing at most 512 certificates. These certificates are added to the connector's trust roots; system roots remain available. This does not install a CA into the OS trust store.

Only the absolute file path is saved in connection metadata. Rust reads and validates the file on each connection/test; certificate bytes never pass through the frontend. Keep the file available when reconnecting. Removing or replacing it affects subsequent connections. Existing sessions keep their established TLS configuration.

The equivalent URL option is `sslrootcert`, alongside `sslmode=require` for PostgreSQL or `tls=required` for MySQL/MariaDB. URL-encode the absolute file path, including spaces and Windows drive paths. Embedded server passwords are removed from stored URLs and handled separately through the OS keychain or session-only password field.

Certificate and hostname checks cannot be disabled. In Klyndb, PostgreSQL `sslmode=require` still verifies both; it does not adopt libpq's weaker interpretation of that mode. A custom CA cannot be combined with plaintext or fallback modes.

## Connection deadline

Expand **Network** in a server connection to choose a connection timeout from **1 to 300 seconds** (default **10**). It is saved as `connect_timeout` in the URL and applies to PostgreSQL, MySQL and MariaDB. For example, `postgresql://user@host/database?connect_timeout=45` or `mysql://user@host/database?connect_timeout=45` keeps default verified TLS while allowing a slower connection.

The application bounds connection setup, including SSH when enabled, DNS/socket attempts, TLS/authentication and initial session setup. Test connection uses the same total deadline through its probe and disconnect. The deadline starts before saved credential lookup and includes the OS credential wait, the serialized creation gate and network setup. Credential reads run off the async runtime; if an OS authorization prompt remains unanswered, the attempt returns an actionable error. The OS read itself cannot be forcibly cancelled: one pending reader keeps its permit until authorization finishes, so repeated attempts do not accumulate lookup threads. Resolve any Keychain prompt before retrying, or enter the required session-only passwords/passphrases in Edit connection. A failed attempt is not added as an active session. This does not change query timeout, metadata deadlines or cancellation's cleanup bound. An existing saved URL without this option keeps the ten-second default; no state migration is needed. Invalid, duplicate, zero or unbounded timeout options are rejected.

## Trusted local development

Use `sslmode=disable` (PostgreSQL) or `tls=disabled` (MySQL/MariaDB) only for a trusted local server. PostgreSQL also accepts `sslmode=prefer`, which can fall back to plaintext; it is an explicit opt-out from the default TLS requirement. Remove the custom CA before selecting either option. A successful connection test alone does not prove encryption when a fallback/plaintext mode was selected.

## Reconnect a saved connection

Choose **Reconnect** beside an open connection, or **Reconnect · connection name** in the command palette. Review the confirmation before proceeding: reconnect closes the old session, rolls back uncommitted transactions and resets temporary objects/session settings. Finish running queries or edits, apply or discard staged rows, and close import/diagram dialogs first. SQL tabs and completed results remain available; their old table results become read-only until you reopen/run the table in the new session. No SQL is automatically replayed.

Rust cancels and awaits native import cleanup, cancels old query jobs and closes the database session/SSH tunnel before opening a new one with the saved settings. The normal credential lookup, connection deadline, read-only and TLS/SSH verification rules still apply. Session-only passwords are not retained for automatic reuse: if needed, enter them again through **Edit connection → Save & connect**. If reconnect fails, the connection remains closed; review the error, correct the settings and connect again. The app does not silently keep the old transaction alive.

The four-engine core contract in `crates/core/tests/reconnect.rs` verifies native session replacement, rollback, temporary-table reset, retained result export, query/import cancellation and recovery after a failed reconnect. Native desktop interaction and Windows/Linux checks are tracked separately in [validation](VALIDATION.md).


## Client certificates (mutual TLS)

When a server requires client authentication, choose a **Client identity** in TLS & certificates. Use a PKCS#12 `.p12` / `.pfx` archive containing your client certificate, private key and intermediate chain. Enter its separate **Certificate password**, test, then save and connect. PostgreSQL, MySQL and MariaDB support this flow.

You can store the certificate password in the OS keychain, independently of the database password, or use it only for the immediate connection. A saved identity with a stored password reconnects without entering it again. To use a protected archive without remembering its password, reopen Edit connection, enter the password and choose Save & connect when reconnecting. Leaving the password blank uses an existing keychain entry, or an empty password if none exists. Removing the identity or deleting its saved connection removes the associated stored certificate password. Duplicating a connection copies the identity path, without copying passwords.

The identity path is stored as the URL option `sslidentity`. Passwords are separate IPC arguments and keychain entries; they are rejected as URL parameters. Identity files have the same 1 MiB regular-file bound as CA files. Rust validates the archive using the platform TLS implementation. Klyndb-managed file buffers and transient passwords are zeroized when dropped; platform TLS/native driver libraries retain the identity and connection options needed by the live session. The private key never crosses IPC or enters local connection/workspace/history storage. Protect the identity file as a credential; Klyndb does not copy it into application state.

A client identity requires verified TLS without plaintext fallback, even when using system trust roots instead of a custom CA. The server must trust the client issuer and authorize the certificate identity. PostgreSQL certificate authentication also checks the certificate name or configured mapping. Certificate format/encryption support follows the native TLS provider; standalone PEM client keys and identity-file creation/conversion remain follow-ups.

[SSH tunnels/bastions](SSH.md) retain the original database hostname for TLS verification. Proxy and multi-hop configuration remain pending. A CA file provides server trust; it is separate from your client identity.

## Local validation

`crates/core/tests/tls.rs` tests disposable TLS-enabled PostgreSQL, MySQL and MariaDB servers. It verifies encrypted sessions, queries, metadata, cancellation and session reuse; it rejects unknown CAs, wrong hostnames, invalid/missing certificate files and custom-CA plaintext configurations. The test uses a PEM bundle with an unrelated CA preceding the correct CA, and verifies the correct root in DER form. An ordinary local test covers empty, oversized, malformed and relative CA files.

Set `KLYNDB_TEST_TLS_CERT_DIR` to an absolute fixture directory containing `ca.pem`, `other-ca.pem`, `bundle.pem` and `invalid.pem` (leave `missing.pem` absent). The server certificate must have DNS SAN `localhost` without an IP SAN for the wrong-hostname check. Set any of `KLYNDB_TEST_TLS_POSTGRES_URL`, `KLYNDB_TEST_TLS_MYSQL_URL` and `KLYNDB_TEST_TLS_MARIADB_URL` to disposable local services using that certificate. The contract creates and removes its own UUID table. `scripts/check.sh` runs configured TLS contracts and explicitly reports skipped ones. Never point these tests at production databases.

Mutual TLS contracts additionally use `KLYNDB_TEST_MTLS_POSTGRES_URL`, `KLYNDB_TEST_MTLS_MYSQL_URL` and `KLYNDB_TEST_MTLS_MARIADB_URL`. The fixture user must require a client certificate: PostgreSQL cert authentication for CN `klyndb_mtls`, and MySQL/MariaDB REQUIRE X509. The certificate directory must contain `client.p12` (trusted clientAuth chain), `wrong-client.p12` (untrusted issuer), with the synthetic fixture password `klyndb-fixture-only`. These disposable test credentials are not application secrets. Contracts reject missing/wrong identities and incorrect archive passwords, verify authenticated encryption and exercise metadata, cancellation, session reuse and reconnect.
