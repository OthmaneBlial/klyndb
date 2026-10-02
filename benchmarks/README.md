# Reproducible measurements

Build and run the Rust disk-streaming benchmark in release mode:

```sh
cargo build --release -p klyndb-core --example stream_bench
target/release/examples/stream_bench 100000
target/release/examples/stream_bench 1000000
```

It checks the actual result count and reads the last page. It reports time to first buffered row, total rows/second and last-page latency. Use `/usr/bin/time -l` on macOS or `/usr/bin/time -v` on Linux on the executable itself for peak process memory; do not measure Cargo's compilation memory as app memory.

These backend measurements do not prove desktop startup, idle WebView memory, scroll FPS, multi-connection behavior or comparative DBeaver performance. Those remain separate required measurements in ROADMAP.md.

The initial frontend bundle measured 927 kB JavaScript (288 kB gzip). Loading CodeMirror and SQL formatting on demand reduces the initial workbench JavaScript to about 264 kB (82 kB gzip); the editor chunk is about 401 kB (131 kB gzip), formatter 262 kB (75 kB gzip). Exact values change with lockfiles and features. Reproduce with `npm run build` in apps/desktop. Editor resident memory and Monaco comparison remain unmeasured; no claim of editor performance superiority is made.
