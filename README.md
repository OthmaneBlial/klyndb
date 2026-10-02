<p align="center">
  <img src="docs/assets/hero.svg" alt="Klyndb — a free, open-source alternative to DBeaver" width="100%" />
</p>

<p align="center">
  <strong>The free, open-source database client for a cleaner everyday workflow.</strong><br />
  Write SQL. Explore schemas. Edit safely. Keep your data on your machine.
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-93d0b5?style=flat-square&labelColor=1c2225" alt="MIT license" /></a>
  <img src="https://img.shields.io/badge/core-Rust-93d0b5?style=flat-square&labelColor=1c2225" alt="Native Rust core" />
  <img src="https://img.shields.io/badge/desktop-Tauri_2-93d0b5?style=flat-square&labelColor=1c2225" alt="Tauri 2 desktop" />
  <img src="https://img.shields.io/badge/status-preview-e8c587?style=flat-square&labelColor=1c2225" alt="Development preview" />
</p>

<p align="center">
  <a href="#get-started">Get started</a> ·
  <a href="#what-you-can-do-today">Features</a> ·
  <a href="docs/COMPATIBILITY.md">Database support</a> ·
  <a href="ROADMAP.md">Roadmap</a> ·
  <a href="CONTRIBUTING.md">Contribute</a>
</p>

---

**Looking for a free, open-source alternative to DBeaver?** Klyndb brings SQL editing, database exploration, streamed results and safe table editing into a focused desktop workspace. It uses a **native Rust core and your system WebView**, built with Tauri 2 and React.

**No Electron. No account. No mandatory cloud. No query telemetry.**

<p align="center">
  <img src="docs/assets/workbench-macos.jpg" alt="Actual Klyndb macOS application with connected SQLite and MySQL databases, SQL tabs and a 10,000-row SQLite result" width="100%" />
  <br /><sub>Captured from the native macOS app with 10,000 synthetic test records in a local validation database.</sub>
</p>

## What you can do today

| Your workflow | Klyndb |
| --- | --- |
| **Connect** | PostgreSQL, MySQL, MariaDB and SQLite; connection testing, saved connections, groups, favorites and environment labels. |
| **Explore** | Tables and views, columns, primary keys, indexes, foreign keys and table DDL. |
| **Write SQL** | Multiple tabs, syntax highlighting, dialect-aware formatting, schema completion and statement/selection/batch execution. |
| **Work with results** | Incremental Rust streaming, disk-backed results, a virtualized grid, resizing/reordering, page sort/filter and cell inspection. |
| **Change data safely** | Staged SQLite/PostgreSQL and MySQL/MariaDB InnoDB inserts, updates and deletes; review, bound values and optimistic conflicts. |
| **Understand queries** | Native estimated plans, collapsible trees, raw output, server messages and confirmed runtime analysis where supported. [Plan guide](docs/EXPLAIN.md). |
| **Stay in control** | Cancellation, timeouts, read-only connections, destructive-query confirmations and actual transaction visibility. |
| **Export** | CSV, typed JSON/JSONL, SQL INSERT and Markdown through native save dialogs. |
| **Pick up where you left off** | Restored workspace, SQL history, saved/favorite queries, theme settings and a command palette. |

Server passwords stay in the **OS keychain**. TLS verification is enabled by default. Your queries and schemas stay local. Read [SECURITY.md](SECURITY.md) for the exact security model and local-history behavior.

### Database support, with real evidence

| Database | Queries & schema | Staged grid edits | Verified against |
| --- | --- | --- | --- |
| PostgreSQL | ✓ | ✓ | PostgreSQL 16 |
| MySQL | ✓ | ✓ · InnoDB | MySQL 8.4.11 |
| MariaDB | ✓ | ✓ · InnoDB | MariaDB 13.0.2 |
| SQLite | ✓ | ✓ | Real SQLite files |

These are implemented engines, tested against actual databases. See the [compatibility matrix](docs/COMPATIBILITY.md) for type, export and workflow limits.

**Development preview:** Klyndb is already runnable from source. Imports, SSH tunnels, more drivers, native package validation and broader desktop testing are still in progress. It does not yet cover every DBeaver workflow. The [roadmap](ROADMAP.md) tracks the next working slices and is updated with each meaningful change.

## Built for a lighter database workflow

Database work belongs in Rust: connections, query execution, cancellation, result paging and exports. React handles the interface. Drivers initialize only when you connect; opening the app does not open database sessions.

Large results go to a bounded, temporary disk spool. The interface holds a **500-row page**, rather than copying an entire result into browser memory.

A reproducible SQLite backend baseline retained **1 million rows** at a median **352,393 rows/second**, with **13.50 MiB peak backend-process RSS** on the recorded Apple M2 machine. This measures the backend only, not total desktop memory or a comparison with DBeaver. See the [benchmark methodology and raw samples](benchmarks/README.md). Desktop startup, memory and scrolling measurements are on the roadmap.

## Get started

You need **Rust stable**, **Node.js 22.12+** and the [Tauri platform prerequisites](https://v2.tauri.app/start/prerequisites/). Linux also needs Secret Service/DBus development libraries; remembered passwords require an unlocked OS keychain.

```sh
git clone https://github.com/OthmaneBlial/klyndb.git
cd klyndb/apps/desktop
npm ci
npm run tauri dev
```

Then create a connection, open a table or SQL tab, and run a real query.

| Shortcut | Action |
| --- | --- |
| `Cmd/Ctrl + Enter` | Run the selected SQL or current statement |
| `Cmd/Ctrl + K` | Open the command palette |
| `Shift + Cmd/Ctrl + F` | Format SQL |
| `Cmd/Ctrl + S` | Save a query |

Build a native package with `npm run tauri build`. Platform targets are macOS, Windows and Linux. Current native workflows are verified on macOS; current driver/package verification on Windows and Linux remains pending. Public downloadable releases are not available yet. macOS signing and notarization require Apple credentials.

## Local checks

```sh
./scripts/check.sh
```

Install the audit tools with `cargo install cargo-audit cargo-deny --locked`. The script runs locked frontend installation, lint, typecheck, tests, production build, Rust formatting/Clippy/tests, a native debug build and dependency/license audits.

Set `KLYNDB_TEST_POSTGRES_URL` or `KLYNDB_TEST_MYSQL_URL` to run the real-server contracts against **disposable local databases**. The MySQL contract runs on either MySQL or MariaDB; verify both separately. Never use production databases for integration tests.

**GitHub Actions is disabled by owner instruction. All current CI checks run locally.**

## Help shape the alternative

Try Klyndb on a development database. Report a reproducible issue, request a database workflow, or contribute a complete driver or UI improvement. If this is the kind of open-source database client you want to use, **star the repository** and follow its progress.

[Contributing](CONTRIBUTING.md) · [Architecture](ARCHITECTURE.md) · [Roadmap](ROADMAP.md) · [Validation evidence](docs/VALIDATION.md) · [Full product scope](docs/PRODUCT_SPEC.md)

**MIT licensed.** Original code and branding. Beekeeper Studio is a functional reference; no Beekeeper source or assets are bundled. Third-party dependencies retain their own licenses: [notices and provenance](THIRD_PARTY_NOTICES.md).
