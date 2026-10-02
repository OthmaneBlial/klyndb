# Third-party notices and provenance

Original Klyndb implementation and icon are MIT licensed. Beekeeper Studio is a functional/UX reference. Only its public README and workflow documentation were reviewed; no source code, commercial directories, assets, trademarks, screenshots or translations are included. Its community GPLv3 and separate commercial licensing remain unchanged. A future reuse of GPL source requires a compatible project license and attribution before it is accepted.

## Direct runtime dependencies

| Dependency | License | Purpose |
| --- | --- | --- |
| Tauri / tauri-build / tauri API | MIT OR Apache-2.0 | System-WebView desktop/IPC/build |
| Tokio / tokio-util | MIT | Async scheduling and cancellation |
| rusqlite | MIT | Native SQLite and local state/result storage |
| SQLite | Public domain | Bundled database engine |
| tokio-postgres / postgres-native-tls / native-tls | MIT OR Apache-2.0 | PostgreSQL protocol/TLS |
| serde / serde_json / uuid / async-trait / thiserror / futures-util | MIT OR Apache-2.0 | Serialization and Rust foundations |
| sqlparser | Apache-2.0 | SQL validation and safety checks |
| keyring | MIT OR Apache-2.0 | OS credentials |
| tempfile | MIT OR Apache-2.0 | Private temporary result/export files |
| zeroize / hex / url / percent-encoding | MIT OR Apache-2.0 | Sensitive memory, encoding and URL handling |
| tracing / tracing-subscriber | MIT | Structured diagnostics |
| csv | MIT OR Unlicense | Streaming CSV |
| rfd | MIT | Native user-controlled file dialogs |
| React / React DOM | MIT | Presentation |
| CodeMirror 6 / Lezer | MIT | SQL editing/parser/completion |
| sql-formatter | MIT | Editor formatting |
| Lucide | ISC | Interface icons |
| IBM Plex Sans / Mono | SIL Open Font License 1.1 | Locally bundled typography |

Toolchain dependencies include TypeScript (Apache-2.0), Vite/Vitest/ESLint/Prettier (MIT), and their transitive packages. Locked versions live in Cargo.lock and apps/desktop/package-lock.json. Generate and review the full transitive license inventory before binary releases; this direct dependency table alone is not a completed release audit. License texts remain with the distributed packages and must be included in packaged release notices.

## Locked inventory

`python3 scripts/license_inventory.py` generates docs/DEPENDENCY_LICENSES.md and bundled THIRD_PARTY_LICENSES.txt from the exact Cargo/npm packages. This includes cross-target Rust dependencies and installed npm license texts; optional npm packages for other platforms are recorded by their lockfile declaration. `cargo deny check licenses` accepts the current Rust graph, including the Apache LLVM exception and MPL-2.0 obligations. The bundled notice file is included as a Tauri resource. Regenerate and review after dependency changes and on each target before release.
