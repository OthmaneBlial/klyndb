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

After `npm ci` in apps/desktop, run `node scripts/bench_completion.mjs > completion.json` from the repository root. The script loads the production completion source through the installed Vite tooling, verifies a chained wildcard's output names and records five index-construction samples plus 50 warm completion samples after five warmups. The statement is already parsed and metadata is cached; no server request, WebView rendering or app startup is measured.

| Catalog tables | Median construction | Median warm completion | Warm p95 |
| --- | --- | --- | --- |
| 10 | 0.097 ms | 0.059 ms | 0.185 ms |
| 1,000 | 10.578 ms | 0.151 ms | 0.198 ms |
| 10,000 | 897.045 ms | 0.921 ms | 0.987 ms |

Apple M2/arm64, Node 25.9.0; configured local CI was running concurrently, so the samples include ambient load. [Raw samples, machine and exact source hashes](history/2026-10-03-completion.json) identify the measured source beyond its base commit. Large-catalog index construction is a concrete remaining performance target. These numbers do not prove native typing responsiveness, desktop startup, scrolling or any advantage over another client.
