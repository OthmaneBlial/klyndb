# Reproducible measurements

Build and run the Rust disk-streaming benchmark in release mode:

```sh
cargo build --release -p klyndb-core --example stream_bench
target/release/examples/stream_bench 100000
target/release/examples/stream_bench 1000000
```

It checks the actual result count and reads the last page. It reports time to first buffered row, total rows/second and last-page latency. Use `/usr/bin/time -l` on macOS or `/usr/bin/time -v` on Linux on the executable itself for peak process memory; do not measure Cargo's compilation memory as app memory.

These backend measurements do not prove desktop startup, idle WebView memory, scroll FPS, multi-connection behavior or comparative DBeaver performance. Those remain separate required measurements in ROADMAP.md.

The initial frontend bundle measured 927 kB JavaScript (288 kB gzip). Loading CodeMirror and SQL formatting on demand reduces the initial workbench JavaScript to about 288 kB (88 kB gzip); the editor chunk is about 401 kB (131 kB gzip), formatter 262 kB (75 kB gzip). Exact values change with lockfiles and features. Reproduce with `npm run build` in apps/desktop. Editor resident memory and Monaco comparison remain unmeasured; no claim of editor performance superiority is made.

## Recorded baseline — 2026-10-02

Run `python3 scripts/bench_stream.py --output benchmarks/history/<date>-stream.json` after the release build. It runs five samples at 100k and 1m rows, verifies row counts, and records native peak RSS, commit, lockfile hash and machine information. The first-row measurement is observed by a 1 ms status poll and includes scheduler/lock jitter.

| Rows | Median total | Rows/sec | First buffered row | Last 500 rows | Peak RSS |
| --- | --- | --- | --- | --- | --- |
| 100,000 | 283 ms | 352,579 | 13 ms | 279 us | 13.42 MiB |
| 1,000,000 | 2,837 ms | 352,393 | 16 ms | 293 us | 13.50 MiB |

Measured on Apple M2 / arm64, release code at `0277fa8`. Raw samples and metadata: [history/2026-10-02-stream.json](history/2026-10-02-stream.json). These are backend-only measurements; they do not establish application idle RAM, desktop startup/scrolling targets or superiority to another database client.

## SQL completion baseline — 2026-10-03

After `npm ci` in apps/desktop, run `node scripts/bench_completion.mjs > completion.json` from the repository root. The script loads the production completion source through installed Vite tooling. It now verifies chained CTE columns, native aliases, complete root/schema menus and a first lazy metadata read. Each catalog has five construction samples, 50 measured samples per warm mode after five warmups, and five first-metadata samples. The original receipt below predates these added modes. The statement is already parsed and metadata is cached; no server request, WebView rendering or app startup is measured.

| Catalog tables | Median construction | Median warm completion | Warm p95 |
| --- | --- | --- | --- |
| 10 | 0.097 ms | 0.059 ms | 0.185 ms |
| 1,000 | 10.578 ms | 0.151 ms | 0.198 ms |
| 10,000 | 897.045 ms | 0.921 ms | 0.987 ms |

Apple M2/arm64, Node 25.9.0; configured local CI was running concurrently, so the samples include ambient load. [Raw samples, machine and exact source hashes](history/2026-10-03-completion.json) identify the measured source beyond its base commit. This original receipt identified large-catalog index construction as a concrete performance target; the partitioned implementation below addresses it. These numbers do not prove native typing responsiveness, desktop startup, scrolling or any advantage over another client.

## Partitioned SQL catalogs — 2026-10-03

CodeMirror namespace construction repeatedly scans growing completion lists. Source builds now bound each namespace to 64 tables, merge suggestions in catalog order, and rebuild only the affected namespace after a metadata read. Table-name ambiguity is computed across the entire catalog. The existing dialect parser, identifier quoting and lazy loader remain in use.

| Tables | Setup before / after | First metadata lookup before / after | Root menu before / after | Schema menu before / after |
| --- | --- | --- | --- | --- |
| 10 | 0.134 / 0.181 ms | 0.243 / 0.246 ms | 0.014 / 0.019 ms | 0.026 / 0.032 ms |
| 1,000 | 17.004 / 7.372 ms | 16.829 / 0.620 ms | 0.039 / 0.305 ms | 0.094 / 0.405 ms |
| 10,000 | 1640.566 / 102.614 ms | 1695.215 / 2.648 ms | 0.173 / 4.169 ms | 0.743 / 4.722 ms |

These are medians on Apple M2/arm64 with Node 25.9.0, using the same expanded script and exact ordered-result assertions before and after the change. The after run overlaps a frontend typecheck; ambient machine load is not controlled, so this is a local observation rather than a universal speedup claim. Full-menu merging adds work at lookup time, trading a few milliseconds for much cheaper construction and metadata refresh. Raw arrays include p95 for each warm mode: [before](history/2026-10-03-completion-partitions-before.json), [after](history/2026-10-03-completion-partitions-after.json). Each receipt identifies the exact production source, benchmark and lockfile hashes as well as its base commit.

Metadata is an immediate synthetic fixture callback, and SQL is already parsed. This does not measure database latency, native startup, rendering, process memory or responsiveness while typing; the published Preview 2 predates the change. A Chrome production-editor check uses a 10,000-table synthetic catalog and two existing SQLite metadata fixtures; native Tauri and other platform acceptance remain separate.
