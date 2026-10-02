# Klyndb

A local-first, open-source database workbench built with **Rust + Tauri 2 + React**.

No Electron. No account. No mandatory cloud service. No query telemetry.

Klyndb is in early development. Its goal is to become a fast, lightweight alternative to DBeaver. Performance comparisons have not yet been established.

## Run from source

Install Rust stable, Node.js 22.12+ and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). On Linux, install Secret Service/DBus development libraries as well. An unlocked OS keychain is needed only for remembered server credentials.

```sh
cd apps/desktop
npm ci
npm run tauri dev
```

Build a native package with `npm run tauri build`. macOS distribution signing and notarization require your own Apple credentials. Platform build status is reported by CI; a local macOS build does not prove Windows or Linux behavior.

## Available in the first slice

- SQLite files and PostgreSQL servers; native Rust drivers, verified TLS by default for PostgreSQL.
- Saved connection metadata, groups, favorites, environment labels and OS keychain passwords.
- Lazy connection activation, table/view discovery on connection, on-demand column/index/foreign-key inspection.
- SQL highlighting, statement/selection/batch execution, schema completion, formatting, multiple tabs and multiple result sets.
- Rust cancellation, query timeout, destructive-query confirmation and read-only connection mode.
- Disk-backed incremental results, a virtualized 500-row page, resizing/reordering, page sort/filter, cell/row/column copying and cell inspection.
- Rust CSV, lossless typed JSON/JSONL, SQL INSERT and Markdown export using native save dialogs and atomic file replacement.
- Local SQL history, saved/favorite queries, theme/settings and automatic workspace restoration.

Only implemented engines appear in the connection form. See the [compatibility matrix](docs/COMPATIBILITY.md) for limitations. This is not yet a replacement for all everyday database workflows: table editing, import, SSH, further drivers and release gates remain in progress.

## Checks

```sh
cargo test
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo audit
cd apps/desktop
npm run typecheck
npm run lint
npm test
npm run build
npm audit
```

Use `cargo test -p klyndb-postgres --test integration` with `-- --ignored` and `KLYNDB_TEST_POSTGRES_URL` to run the real-server contract once installed. Never point integration tests at a production database.

## Contribute

Read [CONTRIBUTING.md](CONTRIBUTING.md), [ARCHITECTURE.md](ARCHITECTURE.md), [SECURITY.md](SECURITY.md) and the [roadmap](ROADMAP.md). The full requested product scope is retained in [PRODUCT_SPEC.md](docs/PRODUCT_SPEC.md).

Original implementation and branding. Beekeeper Studio is a functional reference only; no Beekeeper source or assets are bundled. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
