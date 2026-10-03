# Retained native SQL Server client

This directory retains the original crates.io `tiberius` 0.13.0 distribution under its original MIT/Apache licenses. It is independent of the reference database client's implementation. Original authors, source and licenses are preserved.

- Original archive SHA-256: `e07324791de2bdaed058af4aa433a0b5ec5c0beac407a9797419c19805df2415`.
- Original `.cargo_vcs_info.json` source SHA: `7fd75bb7070fe6ea41374698abf78928bb983bda` (the published package records `dirty: true`).
- Original package: <https://crates.io/crates/tiberius/0.13.0>.

Klyndb changes:

1. Attention cancellation continues across an original response end-of-message until DONE_ATTN is decoded. The upstream method ended early before a separate acknowledgement message. The ordinary query stream still ends at its original message boundary. A late cancellation after a fully consumed response returns without sending unnecessary Attention; poisoned connections remain rejected.
2. Decoded row, metadata, return-value, server error/info and environment-change payloads are removed from tracing. Native error codes and payload-free transport events remain.
3. The Rustls backend uses the existing `ring` provider instead of introducing a second crypto implementation. Process-installed providers remain honored; default certificate/hostname verification and TLS 1.2 remain enabled. The original manifest is preserved in Cargo.toml.orig; the normalized build manifest is changed explicitly.

Real SQL Server driver contracts in `crates/drivers/mssql/tests/integration.rs` reproduce the Attention issue via row truncation, executing-query cancellation, result backpressure and consumer loss, and verify a fresh SELECT on the same session afterward. Failed synchronization still closes the wrapper session.
