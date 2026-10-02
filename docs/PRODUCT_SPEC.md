# KLYRA

Build **Klyra**, a production-quality, open-source, cross-platform database client built with **Rust + Tauri**, designed to become a significantly faster, lighter, cleaner and more modern alternative to **DBeaver**.

Functional reference:

https://github.com/beekeeper-studio/beekeeper-studio

Use Beekeeper Studio as a **functional and UX reference**, not as an architecture to port blindly.

The goal is NOT to mechanically translate Electron/Node/Vue code into Rust.

The goal is to understand what makes a serious universal database client useful and then rebuild the product **from first principles** around performance, simplicity, native capabilities, extensibility and excellent UX.

---

# CORE PRODUCT VISION

Klyra should feel like:

> DBeaver's database capabilities  
> + Beekeeper Studio's simplicity  
> + TablePlus-level polish  
> + native-app performance  
> + a modern open-source architecture.

It should launch quickly, consume little RAM, remain responsive with large datasets, and avoid shipping an entire Chromium/Electron runtime.

The project should eventually be credible enough to market publicly as:

> **Klyra — A fast, lightweight, open-source alternative to DBeaver.**

Optimize relentlessly for:

1. Speed
2. Low memory usage
3. Small distribution size
4. Simple UX
5. Database coverage
6. Stability
7. Security
8. Cross-platform behavior
9. Maintainability
10. Extensibility

Do not sacrifice database support merely to make the initial implementation tiny.

Instead design an architecture where capabilities and database drivers can be added without bloating the application unnecessarily.

---

# IMPORTANT: WORK CONTINUOUSLY

Do not stop after:

- analyzing the reference repository
- creating an architecture document
- scaffolding Tauri
- implementing a login screen
- implementing one database
- creating TODO files
- producing mockups

Those are intermediate steps.

**Continue implementing the project.**

Work autonomously through the backlog.

When one task is complete:

1. run it
2. test it
3. fix failures
4. commit it
5. identify the next highest-impact missing capability
6. continue

Do not ask me what to do next when a reasonable technical decision can be made independently.

When multiple implementations are possible, choose the option that best serves Klyra's goals and continue.

Do not leave fake implementations, placeholder buttons, mocked database actions or TODO-only features when they can reasonably be implemented.

A feature is not complete simply because UI exists.

It must actually work.

---

# LEGAL / SOURCE-PROVENANCE RULES

Be very careful with the Beekeeper Studio repository.

Its repository contains community code as well as separately licensed commercial code.

Never use, translate, port, adapt, copy or derive implementation from any:

`src-commercial`

directory or other commercially licensed source.

Do not copy Beekeeper trademarks, logos, icons, branding, screenshots or proprietary assets.

For community source that is directly reused or translated, preserve all required license and attribution obligations.

Keep a clear `THIRD_PARTY_NOTICES.md` / provenance record.

Prefer implementing Klyra's architecture independently based on required behavior rather than doing line-by-line translations.

If the implementation becomes derivative of GPL-covered Beekeeper source, keep the resulting project license compatible with those obligations.

Do not silently relicense third-party code.

Before public release, audit every dependency and source contribution for licensing.

---

# TECHNOLOGY

## Desktop

Use:

- **Tauri 2**
- Rust stable
- Tokio
- modern async Rust
- system WebView

Do NOT use Electron.

Do NOT embed Chromium unless a feature absolutely cannot be implemented otherwise.

---

# FRONTEND

Use:

- React
- TypeScript
- Vite

Keep JavaScript restricted primarily to presentation/UI responsibilities.

Heavy operations belong in Rust.

Frontend responsibilities:

- rendering
- interactions
- state representation
- keyboard shortcuts
- tab management
- editor UI
- virtualized result grids
- forms
- dialogs
- visualizations

Backend responsibilities:

- database connections
- query execution
- connection pooling
- streaming
- schema introspection
- SSH
- TLS
- filesystem
- secure credential handling
- import/export engines
- cancellation
- background work
- driver implementations
- performance-sensitive transformations

Keep Tauri IPC boundaries typed and intentionally small.

---

# CORE RUST ARCHITECTURE

Create a modular Rust workspace rather than one giant crate.

Suggested structure:

```text
klyra/
├── apps/
│   └── desktop/
├── crates/
│   ├── klyra-core/
│   ├── klyra-driver-api/
│   ├── klyra-query/
│   ├── klyra-schema/
│   ├── klyra-connections/
│   ├── klyra-security/
│   ├── klyra-export/
│   ├── klyra-ssh/
│   ├── klyra-bench/
│   └── drivers/
│       ├── postgres/
│       ├── mysql/
│       ├── sqlite/
│       ├── mssql/
│       ├── redis/
│       ├── mongodb/
│       ├── clickhouse/
│       └── duckdb/
├── packages/
│   └── ui/
├── docs/
└── benchmarks/
```

Refine this structure when necessary.

Do not create abstraction merely for abstraction's sake.

---

# UNIVERSAL DATABASE DRIVER API

The driver system is one of the most important architectural components.

Design a stable internal interface conceptually similar to:

```rust
trait DatabaseDriver {
    async fn connect(...);
    async fn disconnect(...);

    async fn execute(...);
    async fn stream(...);
    async fn cancel(...);

    async fn databases(...);
    async fn schemas(...);
    async fn tables(...);
    async fn columns(...);
    async fn indexes(...);
    async fn constraints(...);
    async fn foreign_keys(...);
    async fn views(...);
    async fn routines(...);

    async fn explain(...);

    fn capabilities(&self) -> DriverCapabilities;
}
```

The exact Rust API can differ.

Use capability detection instead of filling the application with:

```text
if postgres
if mysql
if oracle
if mongo
...
```

UI functionality should adapt according to driver capabilities.

Examples:

```text
supports_transactions
supports_schemas
supports_explain
supports_views
supports_functions
supports_procedures
supports_indexes
supports_foreign_keys
supports_users
supports_roles
supports_native_json
supports_streaming
supports_cancel
supports_backup
supports_restore
supports_ssh
supports_tls
supports_document_queries
```

---

# DATABASE SUPPORT

The long-term objective is **very broad database support**.

Start with excellent support for:

### Tier 1

- PostgreSQL
- MySQL
- MariaDB
- SQLite
- Microsoft SQL Server

Then implement:

### Tier 2

- CockroachDB
- Redshift
- TiDB
- DuckDB
- ClickHouse
- Redis
- MongoDB

Then progressively support:

- Oracle
- Cassandra
- ScyllaDB
- Firebird
- LibSQL
- BigQuery
- Snowflake
- DynamoDB
- Trino
- Presto
- SurrealDB
- SAP HANA where practical
- other commonly requested databases

Do not pretend unsupported engines are supported.

Maintain a real feature compatibility matrix.

---

# DRIVER IMPLEMENTATION STRATEGY

Prefer mature native Rust libraries where available.

Possible ecosystem components include, after validating current maintenance/status:

- sqlx
- tokio-postgres
- rust-postgres
- mysql_async
- rusqlite
- tiberius
- mongodb
- redis
- duckdb
- clickhouse
- rustls
- russh / ssh2

Research current alternatives before committing to libraries.

Do not depend on one giant abstraction such as SQLx for capabilities it does not model correctly.

Database-specific functionality matters.

Use native protocols where practical.

Use ODBC/JDBC/native vendor libraries only where necessary and isolate those dependencies.

---

# LAZY LOADING / BINARY ARCHITECTURE

Klyra must support many databases without making every launch heavy.

Design database-specific capabilities so unused systems do not unnecessarily initialize expensive components.

Explore:

- feature-gated drivers
- optional driver modules
- dynamic driver loading
- separate driver processes where justified
- plugin packages

Startup should initialize only what is actually required.

---

# CONNECTION MANAGER

Implement production-quality connection management.

Features:

- saved connections
- connection groups
- favorites
- duplicate connection
- rename
- color labels
- environments:
  - development
  - staging
  - production
- URL parsing
- connection-string import
- connection testing
- reconnect
- keepalive
- connection pooling
- timeout configuration
- read-only connection option
- SSL/TLS configuration
- client certificates
- CA certificates
- SSH tunnels
- bastion hosts
- proxy support where relevant

Credentials must be stored using OS-native secure storage/keychain whenever possible.

Never save passwords unencrypted in plain configuration files.

---

# SQL EDITOR

Build a genuinely excellent SQL editor.

Required:

- syntax highlighting
- multiple tabs
- execute current statement
- execute selection
- execute entire file
- multi-statement execution
- cancellation
- formatting
- SQL dialect awareness
- line numbers
- bracket matching
- autocomplete
- schema-aware autocomplete
- table completion
- column completion
- alias-aware completion
- saved queries
- query history
- favorites
- recent queries
- errors mapped to editor locations
- keyboard-first workflow
- multiple result sets

Consider CodeMirror 6 or Monaco only after measuring startup/memory impact.

Choose based on evidence, not familiarity.

---

# RESULT GRID

The result grid must remain extremely responsive.

Implement:

- virtualization
- incremental streaming
- millions-of-row friendly behavior
- column resizing
- column reordering
- sort
- local filters
- server filters
- copy cell
- copy row
- copy column
- copy as CSV
- copy as JSON
- NULL visualization
- binary visualization
- JSON viewer
- array viewer
- date/time formatting
- number formatting
- editable rows where supported
- insert row
- delete row
- bulk edits
- transaction-aware editing
- pagination
- result limits
- row count
- execution timing

Never load enormous result sets into React state unnecessarily.

Stream data from Rust.

Use bounded buffers/backpressure.

---

# DATABASE EXPLORER

Create a fast schema navigator.

Example:

```text
Connection
├── Databases
│   └── Database
│       ├── Schemas
│       │   └── Schema
│       │       ├── Tables
│       │       ├── Views
│       │       ├── Materialized Views
│       │       ├── Functions
│       │       ├── Procedures
│       │       ├── Types
│       │       └── Sequences
│       ├── Users
│       └── Roles
```

Use lazy loading.

Do not fetch an entire enterprise schema during startup.

Support search/filter.

Schema metadata should be cached intelligently and refreshable.

---

# TABLE INSPECTOR

Opening a table should provide useful sections such as:

- Data
- Columns
- Indexes
- Foreign keys
- Constraints
- Triggers
- DDL
- Statistics

Allow safe creation/editing/deletion where supported.

---

# QUERY EXPLAIN

Support query planning.

Provide:

- raw explain output
- formatted plan
- tree view
- timings
- estimated cost
- actual cost where supported
- rows
- loops
- warnings

Later consider graphical execution-plan visualization.

---

# ER DIAGRAMS

Implement database relationship diagrams.

Requirements:

- tables
- columns
- PKs
- FKs
- relationship lines
- zoom
- pan
- auto-layout
- manual layout
- export image/SVG if practical
- saved diagram state

Large schemas must remain usable.

---

# IMPORT / EXPORT

Implement robust export:

- CSV
- JSON
- JSONL
- SQL INSERT
- Markdown
- clipboard

Later:

- XLSX
- Parquet

Implement imports where practical:

- CSV
- JSON
- SQL

Support streaming imports/exports instead of reading everything into RAM.

---

# DOCUMENT DATABASES

Do not force MongoDB into a relational SQL-shaped UI.

Allow driver-specific experiences.

MongoDB:

- databases
- collections
- documents
- indexes
- JSON/tree viewer
- query execution
- aggregation pipelines
- document editing

Redis:

- key explorer
- type detection
- TTL
- string
- hashes
- lists
- sets
- sorted sets
- streams where practical
- key search
- raw commands

The universal architecture should permit specialized driver UIs.

---

# UX

Klyra must be noticeably simpler than DBeaver.

Avoid endless toolbars, tiny icons and modal overload.

Default layout:

```text
┌──────────────────────────────────────────────────────────────┐
│ toolbar / connection / command palette                     │
├───────────────┬──────────────────────────────────────────────┤
│ Connections   │ tabs                                        │
│               ├──────────────────────────────────────────────┤
│ DB explorer   │ SQL Editor / Table / Diagram                │
│               │                                             │
│               ├──────────────────────────────────────────────┤
│               │ Results / Messages / Explain                │
└───────────────┴──────────────────────────────────────────────┘
```

Prioritize:

- information density
- clear hierarchy
- keyboard navigation
- minimal visual noise
- excellent dark mode
- excellent light mode
- native-feeling interactions
- responsive resizing
- accessibility

Create original branding.

Do not imitate Beekeeper visual assets.

---

# COMMAND PALETTE

Implement a command palette:

`Cmd/Ctrl + K` or similar.

Examples:

```text
Connect to database
New SQL tab
Open table
Search table
Refresh schema
Run query
Format SQL
Export results
Toggle sidebar
Open settings
Switch connection
```

---

# KEYBOARD-FIRST EXPERIENCE

Support shortcuts for the most common actions.

Developers should be able to use Klyra efficiently without touching the mouse constantly.

---

# MULTI-CONNECTION WORKSPACES

Users should be able to have:

```text
PostgreSQL production
MySQL staging
SQLite local
Mongo analytics
Redis cache
```

open simultaneously.

Tabs must clearly show their active connection.

Production environments should be visually distinguishable.

---

# SAFETY

Database clients can destroy production data.

Implement protective features:

- production connection indicator
- optional read-only connections
- destructive-query detection
- configurable confirmation for:
  - DROP
  - TRUNCATE
  - DELETE without WHERE
  - UPDATE without WHERE
- transaction visibility
- connection environment labels

Do not make normal database work annoying, but provide strong safeguards.

---

# SECURITY

Treat database credentials as extremely sensitive.

Requirements:

- OS keychain where available
- TLS verification enabled by default
- avoid logging secrets
- redact connection URLs
- clear sensitive buffers where feasible
- safe error reporting
- strict Tauri permissions
- minimal frontend privileges
- no arbitrary shell execution
- no arbitrary filesystem exposure
- dependency auditing

Run:

```bash
cargo audit
```

and appropriate frontend audits.

---

# PERFORMANCE REQUIREMENTS

Performance is not a marketing claim.

Measure it.

Create reproducible benchmarks for:

### Startup

- cold launch
- warm launch
- time until interactive

### Memory

- idle
- one connection
- five connections
- 100k-row result set
- large schema

### Query

- latency overhead added by Klyra
- streaming throughput
- cancellation latency

### UI

- large grid scrolling
- 100+ tabs
- thousands of schema objects

Track benchmark history.

---

# PERFORMANCE TARGETS

These are targets, not fabricated claims:

- app usable in ~1 second on a modern machine where practical
- idle RAM dramatically below Electron-based database clients
- smooth 60fps scrolling for virtualized grids
- no UI lock while queries run
- near-immediate cancellation
- no full-schema loading at startup
- no full-result copying between Rust and JS unnecessarily

If targets cannot be reached, profile the reason.

Do not simply loosen the target.

---

# PROFILING

Use evidence.

Rust:

- cargo flamegraph where available
- tracing
- criterion
- heap/memory profiling

Frontend:

- React profiler
- browser performance tooling
- bundle analysis

Tauri:

- IPC instrumentation

Optimize measured bottlenecks.

---

# LOCAL STATE

Use a lightweight local database such as SQLite for application state if appropriate.

Store:

- connection metadata excluding secrets
- recent queries
- saved queries
- preferences
- layouts
- workspace state
- history

Use proper migrations.

---

# APPLICATION SETTINGS

Create simple settings for:

- theme
- font
- editor font
- SQL formatter
- query timeout
- row limit
- auto-commit
- result behavior
- keyboard shortcuts
- privacy
- updates
- driver settings

Do not build a giant preferences maze.

---

# UPDATES

Support application updates cleanly through Tauri's updater architecture when appropriate.

Do not compromise security for convenience.

---

# CROSS PLATFORM

Klyra must be a first-class citizen on:

- macOS Apple Silicon
- macOS Intel where practical
- Windows x64
- Linux x64

Later consider:

- Windows ARM64
- Linux ARM64

Do not treat macOS as the only real platform.

---

# PACKAGING

Produce proper releases:

macOS:

- `.dmg`
- `.app`

Windows:

- installer
- portable build if reasonable

Linux:

- AppImage
- `.deb`
- `.rpm` where practical

---

# CI/CD

Use GitHub Actions.

Required:

- Rust formatting
- clippy
- Rust tests
- frontend lint
- TypeScript typechecking
- frontend tests
- integration tests
- builds on macOS
- builds on Windows
- builds on Linux
- dependency audit
- release builds

Cache responsibly.

---

# TESTING

Tests are mandatory.

## Rust unit tests

Core abstractions and parsing.

## Integration tests

Run actual databases with containers where possible:

- PostgreSQL
- MySQL
- MariaDB
- SQL Server
- MongoDB
- Redis
- ClickHouse

## E2E

Test real workflows:

```text
launch
→ create connection
→ connect
→ inspect schema
→ open SQL tab
→ execute query
→ receive results
→ edit data
→ export
→ disconnect
```

Do not consider a driver complete merely because `connect()` succeeds.

---

# ERROR HANDLING

No random panics.

Use structured errors.

Errors presented to users should be understandable.

Preserve useful driver/server information without leaking credentials.

---

# LOGGING

Use structured Rust logging/tracing.

Support:

- info
- debug
- trace

Logs must redact secrets.

Create an easy way for users to export diagnostic information without credentials.

---

# CONTRIBUTOR EXPERIENCE

This is an open-source project.

Make contributing easy.

Create:

- README.md
- CONTRIBUTING.md
- ARCHITECTURE.md
- SECURITY.md
- CODE_OF_CONDUCT.md
- ROADMAP.md
- THIRD_PARTY_NOTICES.md
- issue templates
- PR template

Document how to add a database driver.

Ideally adding a new driver should require implementing the driver API rather than editing dozens of unrelated modules.

---

# README

The README should immediately communicate:

```text
Klyra

The fast, lightweight, open-source database client.

PostgreSQL • MySQL • MariaDB • SQLite • SQL Server • MongoDB • Redis • ...

Built with Rust + Tauri.

No Electron.
No account required.
Local-first.
Cross-platform.
```

Include screenshots once the real UI exists.

Do not invent screenshots.

---

# MARKETING POSITIONING

Position the project as:

> A fast and lightweight open-source alternative to DBeaver.

Secondary comparisons can include:

- Beekeeper Studio
- TablePlus
- DataGrip
- DbVisualizer

Never make unsupported benchmark claims.

If README says:

> 4x faster

there must be a reproducible benchmark proving it.

Prefer statements such as:

> Built around a native Rust core and Tauri rather than Electron.

---

# PRIVACY

Default to local-first behavior.

No account required.

No mandatory cloud service.

No query telemetry.

No schema upload.

If telemetry is ever introduced:

- disabled by default
- explicitly documented
- opt-in

---

# AI

AI is NOT required for the core database client.

Do not turn Klyra into an AI wrapper.

If AI features are eventually added, treat them as optional plugins/features.

Core database functionality must remain excellent without AI.

---

# EXTENSION SYSTEM

Once the core is stable, design an extension architecture.

Possible extensions:

- database drivers
- themes
- exporters
- formatters
- visualization tools

Do not execute arbitrary untrusted native plugins without a security model.

Consider process isolation or WASM for third-party extensions.

---

# BUILD ORDER

Prioritize vertical working slices.

## Milestone 1

Real desktop application:

- Tauri
- React
- Rust core
- connection storage
- PostgreSQL
- SQLite
- schema tree
- SQL editor
- streamed result grid
- query cancellation

## Milestone 2

- MySQL/MariaDB
- table viewer
- editable rows
- import/export
- query history
- saved queries
- SSH
- TLS
- secure credentials

## Milestone 3

- SQL Server
- DuckDB
- ClickHouse
- explain plans
- ER diagrams
- production safety

## Milestone 4

- MongoDB
- Redis
- specialized NoSQL interfaces
- driver capability system maturity

## Milestone 5

Expand database compatibility aggressively.

Do not wait until every database exists before creating a usable release.

Every milestone should result in a genuinely runnable application.

---

# REFERENCE ANALYSIS

Before implementing major areas, inspect the Beekeeper repository to understand:

- user workflows
- database coverage
- connection configuration
- schema navigation
- query workflows
- result grids
- data editing
- import/export
- UX conventions
- edge cases

Create an internal feature matrix:

```text
Feature
Beekeeper behavior
Klyra implementation
Status
Notes
```

But do not spend days documenting instead of coding.

Analysis should directly feed implementation.

---

# IMPROVE, DON'T COPY

When Beekeeper has:

```text
10 configuration controls
```

ask whether Klyra can expose the common 3 first and hide advanced options.

When Beekeeper loads something eagerly, consider lazy loading.

When a workflow requires 4 dialogs, see if Klyra can do it inline.

When a dependency exists only because Electron requires it, remove that assumption entirely.

Use the rewrite as an opportunity to simplify.

---

# QUALITY BAR

Do not ship:

- fake data
- dead controls
- mock backend calls
- placeholder screenshots
- non-working menus
- disabled features presented as complete
- enormous files with unrelated responsibilities
- unsafe `unwrap()` everywhere
- silent failures
- secrets in logs
- blocking DB work on the UI thread

Prefer boring, maintainable engineering.

---

# AUTONOMOUS ITERATION LOOP

Continuously execute this loop:

```text
inspect existing state
        ↓
identify highest-value missing capability
        ↓
implement smallest complete vertical slice
        ↓
compile
        ↓
run tests
        ↓
run application
        ↓
manually verify behavior where possible
        ↓
profile if performance-sensitive
        ↓
fix regressions
        ↓
update docs
        ↓
commit
        ↓
continue
```

Do not stop merely because a milestone is reached if useful work remains and the environment allows continuing.

---

# BUG POLICY

If you discover a bug while implementing another feature:

- fix serious regressions immediately
- add a regression test where practical
- continue the original work

Do not accumulate obvious broken behavior.

---

# GIT

Use clean commits.

Examples:

```text
feat(postgres): add streaming query execution
feat(grid): virtualize large result sets
feat(mysql): add schema introspection
feat(ssh): add tunneled connections
perf(ipc): stream batches instead of row-by-row events
fix(sqlite): preserve null values during editing
docs(drivers): document driver implementation API
```

Commit meaningful working states.

---

# GITHUB

If GitHub credentials/access are available:

1. verify whether the final project name is reasonably available
2. create the repository under my account
3. use `klyra` as the preferred repository name
4. initialize the repository properly
5. create/update the README
6. push work regularly
7. use `main` as the default development branch unless the environment dictates otherwise
8. create tagged releases once the application is legitimately usable

Do not force-push destructively over unrelated existing work.

---

# NAMING CHECK BEFORE PUBLIC LAUNCH

"Klyra" is the current working name.

Before publicly branding the project:

- search GitHub
- search package registries
- search crates.io
- search npm
- search major search engines
- check obvious domain conflicts
- check obvious software trademark conflicts

If there is a significant collision, choose a similarly short, distinctive replacement and update branding consistently.

Do not spend excessive implementation time on naming.

---

# DEFINITION OF SUCCESS

Klyra is successful when a developer can install it and genuinely replace a large portion of their everyday DBeaver/Beekeeper workflow with it.

A successful early version must let the user:

```text
install Klyra
↓
launch almost immediately
↓
create a real DB connection
↓
browse schemas and tables
↓
open and inspect data
↓
write SQL
↓
execute queries
↓
stream large results smoothly
↓
edit data safely
↓
export results
↓
switch between databases
↓
close the app without losing workspace state
```

And the application must feel:

- fast
- lightweight
- intentional
- stable
- modern

not like an unfinished demo.

---

# FINAL PRINCIPLE

Whenever forced to choose between:

**more architecture** and **a complete working feature**,

prefer the complete working feature unless architecture is necessary for future database portability.

Whenever forced to choose between:

**cleverness** and **performance + maintainability**,

choose performance + maintainability.

Whenever forced to choose between:

**copying the reference implementation** and **building a simpler native Rust design**,

build the native Rust design.

Build something developers will want to install, star, contribute to and keep open all day.

**Make Klyra the database client people recommend when someone asks:**

> "Is there a fast, lightweight, open-source alternative to DBeaver?"