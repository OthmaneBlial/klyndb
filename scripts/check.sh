#!/usr/bin/env bash
# Full local CI. Requires Rust stable, Node 22.12+, cargo-audit and cargo-deny.
set -euo pipefail
cd "$(dirname "$0")/.."

npm --prefix apps/desktop ci
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop test
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
cargo build --locked -p klyndb-desktop
cargo audit
cargo deny check licenses
