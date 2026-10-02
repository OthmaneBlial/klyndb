# Third-party notices and provenance

Original Klyndb implementation and icon are MIT licensed. Beekeeper Studio is a functional/UX reference. Only its public README and workflow documentation were reviewed; no source code, commercial directories, assets, trademarks, screenshots or translations are included. Its community GPLv3 and separate commercial licensing remain unchanged. A future reuse of GPL source requires a compatible project license and attribution before it is accepted.

## Direct runtime dependencies

| Dependency | License | Purpose |
| --- | --- | --- |
| Tauri / tauri-build / tauri API | MIT OR Apache-2.0 | System-WebView desktop/IPC/build |
| Tokio / tokio-util | MIT | Async scheduling and cancellation |
| async-ssh2-lite / ssh2 | MIT OR Apache-2.0 | Native SSH authentication and TCP forwarding |
| libssh2 / libssh2-sys | BSD-3-Clause / MIT OR Apache-2.0 | Bundled SSH protocol library; native notices retained |
| OpenSSL / openssl-src | Apache-2.0 / MIT OR Apache-2.0 | Bundled SSH cryptography, including RSA |
| zlib | Zlib | Native SSH compression dependency |
| rusqlite | MIT | Native SQLite and local state/result storage |
| SQLite | Public domain | Bundled database engine |
| duckdb / libduckdb-sys / DuckDB | MIT, plus original native third-party terms | Embedded DuckDB 1.5.6, Rust wrapper 1.10506.0; pinned native notices retained |
| tokio-postgres / postgres-native-tls / native-tls | MIT OR Apache-2.0 | PostgreSQL protocol; verified server TLS and client identities |
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

The independent SSH test server uses russh (Apache-2.0) with ring and without its RustCrypto RSA feature. Klyndb does not reuse russh server implementation source. SSH forwarding corrects the native provider's read-buffer-discarding flush semantics in an original stream adapter; API behavior was checked against the installed ssh2/async-ssh2-lite sources and [libssh2 documentation](https://libssh2.org/libssh2_channel_flush_ex.html).

Toolchain dependencies include TypeScript (Apache-2.0), Vite/Vitest/ESLint/Prettier (MIT), and their transitive packages. Locked versions live in Cargo.lock and apps/desktop/package-lock.json. Generate and review the full transitive license inventory before binary releases; this direct dependency table alone is not a completed release audit. License texts remain with the distributed packages and must be included in packaged release notices.

## Locked inventory

`python3 scripts/license_inventory.py` generates docs/DEPENDENCY_LICENSES.md and bundled THIRD_PARTY_LICENSES.txt from the exact Cargo/npm packages. This includes cross-target Rust dependencies and installed npm license texts; optional npm packages for other platforms are recorded by their lockfile declaration. `cargo deny check licenses` accepts the current Rust graph, including the Apache LLVM exception and MPL-2.0 obligations. The bundled notice file is included as a Tauri resource. Regenerate and review after dependency changes and on each target before release.

The collector retains LICENSE/LICENCE, COPYING, COPYRIGHT, NOTICE/NOTICES, UNLICENSE and OFL text files, including the separately retained native SSH/cryptography/compression notices. Undecodable text stops collection instead of being silently dropped. `python3 scripts/test_license_inventory.py` checks filename variants and notice retention; it runs in local CI. Package contents still require target-specific verification before a public release.

DuckDB is integrated through the public `duckdb` Rust wrapper, independently of the reference client. The generated native archive in `libduckdb-sys` 1.10506.0 identifies DuckDB 1.5.6, source ID `069cc9f9b5`. It omits standalone native notice files, so [the retained native notices](third_party/duckdb-1.5.6/NOTICES.txt) include the original texts for the source directories present in that archive, fetched from upstream commit `069cc9f9b5be802405797faecc284961b07c70ef`. The license collector includes them in the packaged notice resource and requires an explicit notice update for a different native version. Auto-install, automatic extension loading and external file/network SQL access are disabled in this driver slice.

DuckDB's build-time `ureq` dependency uses `webpki-roots` 1.0.9 certificate data under [CDLA-Permissive-2.0](https://cdla.dev/permissive-2-0/). Sharing that data requires making the license text available alongside it; the full original package license is retained in `THIRD_PARTY_LICENSES.txt`. The license policy allows this reviewed permissive data license.

ClickHouse uses the pinned native Rust `klickhouse` 0.15.3 client (MIT OR Apache-2.0), independently of the reference client. The original source, authorship and licenses are retained in [third_party/klickhouse-0.15.3](third_party/klickhouse-0.15.3/KLYNDB_PATCH.md), with duplicate-column rejection, a read-only handshake probe and payload-free diagnostics documented there. The license collector includes this path dependency and its exact license texts. Native LZ4 compression retains its original upstream distribution and library licenses in the packaged notice resource.
