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

## Trusted local development

Use `sslmode=disable` (PostgreSQL) or `tls=disabled` (MySQL/MariaDB) only for a trusted local server. PostgreSQL also accepts `sslmode=prefer`, which can fall back to plaintext; it is an explicit opt-out from the default TLS requirement. Remove the custom CA before selecting either option. A successful connection test alone does not prove encryption when a fallback/plaintext mode was selected.

Client certificate authentication (mTLS), SSH tunnels/bastions and proxy configuration remain pending. A CA file is not a client identity or private key.

## Local validation

`crates/core/tests/tls.rs` tests disposable TLS-enabled PostgreSQL, MySQL and MariaDB servers. It verifies encrypted sessions, queries, metadata, cancellation and session reuse; it rejects unknown CAs, wrong hostnames, invalid/missing certificate files and custom-CA plaintext configurations. The test uses a PEM bundle with an unrelated CA preceding the correct CA, and verifies the correct root in DER form. An ordinary local test covers empty, oversized, malformed and relative CA files.

Set `KLYNDB_TEST_TLS_CERT_DIR` to an absolute fixture directory containing `ca.pem`, `other-ca.pem`, `bundle.pem` and `invalid.pem` (leave `missing.pem` absent). The server certificate must have DNS SAN `localhost` without an IP SAN for the wrong-hostname check. Set any of `KLYNDB_TEST_TLS_POSTGRES_URL`, `KLYNDB_TEST_TLS_MYSQL_URL` and `KLYNDB_TEST_TLS_MARIADB_URL` to disposable local services using that certificate. The contract creates and removes its own UUID table. `scripts/check.sh` runs configured TLS contracts and explicitly reports skipped ones. Never point these tests at production databases.
