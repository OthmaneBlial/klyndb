#!/usr/bin/env bash
# Full local CI. Requires Rust stable, Node 22.12+, Python 3, cargo-audit and cargo-deny.
set -euo pipefail
cd "$(dirname "$0")/.."

npm --prefix apps/desktop ci
npm --prefix apps/desktop run lint
npm --prefix apps/desktop run typecheck
npm --prefix apps/desktop test
node scripts/check_site.mjs
python3 scripts/test_license_inventory.py
npm --prefix apps/desktop run build
npm --prefix apps/desktop audit
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets -- -D warnings
# Keep the full workspace graph: selecting individual core targets recompiles
# the bundled DuckDB C++ library with a different host dependency feature set.
test_artifacts=$(mktemp)
trap 'rm -f "$test_artifacts"' EXIT
cargo test --locked --workspace --no-run --message-format=json > "$test_artifacts"
workspace_test() {
  python3 - "$test_artifacts" "$@" <<'PY'
import json
from pathlib import Path
import subprocess
import sys

manifest, source, *arguments = sys.argv[1:]
source = str(Path(source).resolve())
executables = set()
for line in Path(manifest).read_text().splitlines():
    artifact = json.loads(line)
    if artifact.get("reason") == "compiler-artifact" and artifact.get("executable") and artifact["target"]["src_path"] == source:
        executables.add(artifact["executable"])
if len(executables) != 1:
    raise SystemExit(f"Expected one workspace test executable for {source}, got {len(executables)}")
raise SystemExit(subprocess.run([executables.pop(), *arguments]).returncode)
PY
}
cargo test --locked --workspace
if [[ -n "${KLYNDB_TEST_POSTGRES_URL:-}" ]]; then
  workspace_test crates/drivers/postgres/tests/integration.rs --ignored
else
  echo 'PostgreSQL integration skipped: set KLYNDB_TEST_POSTGRES_URL to a disposable test server.'
fi
if [[ -n "${KLYNDB_TEST_MYSQL_URL:-}" ]]; then
  workspace_test crates/drivers/mysql/tests/integration.rs --ignored
else
  echo 'MySQL/MariaDB integration skipped: set KLYNDB_TEST_MYSQL_URL to a disposable test server.'
fi
if [[ -n "${KLYNDB_TEST_CLICKHOUSE_URL:-}" ]]; then
  workspace_test crates/drivers/clickhouse/tests/integration.rs real_clickhouse_workflow --ignored
  if [[ -n "${KLYNDB_TEST_CLICKHOUSE_DELAY_HOST:-}" ]]; then
    workspace_test crates/core/tests/plans.rs clickhouse_plan_cancellation_and_deadline_during_schema_inference --ignored --nocapture
  else
    echo 'ClickHouse planning HTTP deadline skipped: set KLYNDB_TEST_CLICKHOUSE_DELAY_HOST to a server-reachable local fixture host.'
  fi
else
  echo 'ClickHouse integration skipped: set KLYNDB_TEST_CLICKHOUSE_URL to a disposable native TCP server.'
fi
if [[ -n "${KLYNDB_TEST_MSSQL_URL:-}" ]]; then
  : "${KLYNDB_TEST_MSSQL_PASSWORD:?Set KLYNDB_TEST_MSSQL_PASSWORD for the disposable SQL Server}"
  workspace_test crates/drivers/mssql/tests/integration.rs real_sql_server_workflow --ignored
  workspace_test crates/drivers/mssql/tests/integration.rs real_sql_server_catalog --ignored
  workspace_test crates/drivers/mssql/tests/integration.rs real_sql_server_editing --ignored
  workspace_test crates/drivers/mssql/tests/integration.rs real_sql_server_imports --ignored
  if [[ -n "${KLYNDB_TEST_TLS_CERT_DIR:-}" ]]; then
    workspace_test crates/drivers/mssql/tests/integration.rs real_sql_server_verified_tls --ignored
  fi
else
  echo 'SQL Server integration skipped: set KLYNDB_TEST_MSSQL_URL and KLYNDB_TEST_MSSQL_PASSWORD.'
fi
if [[ -n "${KLYNDB_TEST_TLS_CLICKHOUSE_URL:-}" ]]; then
  : "${KLYNDB_TEST_TLS_CERT_DIR:?Set KLYNDB_TEST_TLS_CERT_DIR for ClickHouse TLS contracts}"
  workspace_test crates/drivers/clickhouse/tests/integration.rs real_clickhouse_verified_tls_and_native_readonly_profile --ignored
else
  echo 'ClickHouse TLS integration skipped: set KLYNDB_TEST_TLS_CLICKHOUSE_URL and KLYNDB_TEST_TLS_CERT_DIR.'
fi
for entry in POSTGRES:postgres MYSQL:mysql MARIADB:mariadb; do
  variable="KLYNDB_TEST_${entry%%:*}_URL"
  if [[ -n "${!variable:-}" ]]; then
    workspace_test crates/core/tests/connect.rs "${entry#*:}_delayed_connection" --ignored
  else
    echo "Delayed-handshake integration skipped: set $variable to a disposable server."
  fi
  variable="KLYNDB_TEST_TLS_${entry%%:*}_URL"
  if [[ -n "${!variable:-}" ]]; then
    : "${KLYNDB_TEST_TLS_CERT_DIR:?Set KLYNDB_TEST_TLS_CERT_DIR for TLS contracts}"
    workspace_test crates/core/tests/tls.rs "${entry#*:}_verified_tls" --ignored
  else
    echo "TLS integration skipped: set $variable and KLYNDB_TEST_TLS_CERT_DIR."
  fi
  variable="KLYNDB_TEST_MTLS_${entry%%:*}_URL"
  if [[ -n "${!variable:-}" ]]; then
    : "${KLYNDB_TEST_TLS_CERT_DIR:?Set KLYNDB_TEST_TLS_CERT_DIR for mTLS contracts}"
    workspace_test crates/core/tests/tls.rs "${entry#*:}_mutual_tls" --ignored
    if [[ -n "${KLYNDB_TEST_SSH_DIR:-}" ]]; then
      : "${KLYNDB_TEST_SSH_USER:?Set the disposable SSH fixture user}"
      : "${KLYNDB_TEST_SSH_FINGERPRINT:?Set the independently verified SSH host fingerprint}"
      workspace_test crates/core/tests/ssh.rs "${entry#*:}_ssh" --ignored
    else
      echo "SSH integration skipped: set KLYNDB_TEST_SSH_DIR, KLYNDB_TEST_SSH_USER and KLYNDB_TEST_SSH_FINGERPRINT."
    fi
  else
    echo "Mutual TLS integration skipped: set $variable and KLYNDB_TEST_TLS_CERT_DIR."
  fi
done
if [[ "${KLYNDB_TEST_KEYCHAIN:-}" == 1 ]]; then
  workspace_test crates/core/src/lib.rs ssh_keychain_credentials --ignored
else
  echo 'SSH keychain scope contract skipped: set KLYNDB_TEST_KEYCHAIN=1 on a disposable development session.'
fi
cargo build --locked --workspace --all-targets
cargo audit
cargo deny check licenses
