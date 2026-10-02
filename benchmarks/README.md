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
