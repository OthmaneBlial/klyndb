#!/usr/bin/env bash
# Full local CI. Requires Rust stable, Node 22.12+, cargo-audit and cargo-deny.
set -euo pipefail
cd "$(dirname "$0")/.."

npm --prefix apps/desktop ci
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop test
node scripts/check_site.mjs
npm --prefix apps/desktop run build
npm --prefix apps/desktop audit
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
if [[ -n "${KLYNDB_TEST_POSTGRES_URL:-}" ]]; then
  cargo test --locked -p klyndb-postgres --test integration -- --ignored
else
  echo 'PostgreSQL integration skipped: set KLYNDB_TEST_POSTGRES_URL to a disposable test server.'
fi
if [[ -n "${KLYNDB_TEST_MYSQL_URL:-}" ]]; then
  cargo test --locked -p klyndb-mysql --test integration -- --ignored
else
  echo 'MySQL/MariaDB integration skipped: set KLYNDB_TEST_MYSQL_URL to a disposable test server.'
fi
for entry in POSTGRES:postgres MYSQL:mysql MARIADB:mariadb; do
  variable="KLYNDB_TEST_${entry%%:*}_URL"
  if [[ -n "${!variable:-}" ]]; then
    cargo test --locked -p klyndb-core --test connect "${entry#*:}_delayed_connection" -- --ignored
  else
    echo "Delayed-handshake integration skipped: set $variable to a disposable server."
  fi
  variable="KLYNDB_TEST_TLS_${entry%%:*}_URL"
  if [[ -n "${!variable:-}" ]]; then
    : "${KLYNDB_TEST_TLS_CERT_DIR:?Set KLYNDB_TEST_TLS_CERT_DIR for TLS contracts}"
    cargo test --locked -p klyndb-core --test tls "${entry#*:}_verified_tls" -- --ignored
  else
    echo "TLS integration skipped: set $variable and KLYNDB_TEST_TLS_CERT_DIR."
  fi
  variable="KLYNDB_TEST_MTLS_${entry%%:*}_URL"
  if [[ -n "${!variable:-}" ]]; then
    : "${KLYNDB_TEST_TLS_CERT_DIR:?Set KLYNDB_TEST_TLS_CERT_DIR for mTLS contracts}"
    cargo test --locked -p klyndb-core --test tls "${entry#*:}_mutual_tls" -- --ignored
  else
    echo "Mutual TLS integration skipped: set $variable and KLYNDB_TEST_TLS_CERT_DIR."
  fi
done
cargo build --locked -p klyndb-desktop
cargo audit
cargo deny check licenses
