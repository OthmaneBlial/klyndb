# Functional reference matrix

Reviewed the public README and user workflow documentation of [Beekeeper Studio](https://github.com/beekeeper-studio/beekeeper-studio), reference commit `49a48c235afd42586460b89014b56579dccc93a8` on 2026-10-02. No implementation files or commercial sources were used.

| Feature | Reference workflow | Klyndb implementation | Status / next check |
| --- | --- | --- | --- |
| Connections | Saved database configurations and tabbed sessions | Rust metadata DB, OS keychain, explicit environment, independent sessions, isolated draft tests, shared configurable connection deadlines | SQLite/PostgreSQL/MySQL/MariaDB implemented; verified TLS/custom CA implemented; PKCS#12 client identities implemented; single-bastion SSH with pinned host keys and separate credentials implemented; native Windows/Linux checks pending |
| SQL | Highlighting, completion, selections, tabs and history | CodeMirror 6, native drivers, AST safety, disk history; executable comments rejected | Native macOS workflows verified; alias completion and error locations pending |
| Table view | Open table, inspect cells, sort/filter/edit records | On-demand inspection, paged grid, staged insert/update/delete with savepoint rollback | SQLite/PostgreSQL and MySQL/MariaDB InnoDB editing; native macOS editing checks passed; database filters/sort/pages implemented with real-engine contracts; packaged macOS SQLite filtering/sorting/next/previous and visible SQL verified |
| Structure | Inspect columns, keys and table definitions | Native constraints/user triggers, PostgreSQL firing modes, SQLite/MySQL DDL | Four-engine contracts pass; packaged macOS SQLite constraint DDL/trigger expansion and MySQL constraint table/DDL verified |
| Export | Query/table results into files or clipboard | Rust streaming serializers and native save dialogs | CSV/JSON/JSONL/SQL/Markdown implemented |
| Import | Choose file, map fields, preview/import | Independent bounded CSV/JSON parsers, private snapshots, typed mapping and native transaction-backed inserts | CSV and JSON object/native-export arrays plus SQL scripts, native file selection, review/confirmation and progress/cancel implemented; SQL preserves script transactions and reports partial effects; see [import contract](IMPORTS.md) |
| Query plans | Inspect EXPLAIN and runtime plans alongside results | Native driver formats, Rust tree normalization, raw output and server messages | All four backend contracts verified; native macOS MySQL/SQLite workflows passed |
| Diagrams | Explore tables and relationships across schemas | Native typed foreign keys, SVG workspace, manual/grid layout, pan/zoom, local layouts and native SVG export | Four-engine composite/self-FK contracts pass; native and large-catalog checks are recorded in validation evidence |
| NoSQL | Specialized engine behavior | Separate document/key experiences planned | Pending |

References are behavior only. No branded icons, logos, screenshots or source were copied. Analyze each major area just before implementing it, and update this matrix from actual behavior.
