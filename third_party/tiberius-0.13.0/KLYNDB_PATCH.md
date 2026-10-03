# Retained native SQL Server client

This directory retains the original crates.io `tiberius` 0.13.0 distribution under its original MIT/Apache licenses. It is independent of the reference database client's implementation. Original authors, source and licenses are preserved.

- Original archive SHA-256: `e07324791de2bdaed058af4aa433a0b5ec5c0beac407a9797419c19805df2415`.
- Original `.cargo_vcs_info.json` source SHA: `7fd75bb7070fe6ea41374698abf78928bb983bda` (the published package records `dirty: true`).
- Original package: <https://crates.io/crates/tiberius/0.13.0>.

Klyndb changes:

1. Attention cancellation continues across an original response end-of-message until DONE_ATTN is decoded. The upstream method ended early before a separate acknowledgement message. The ordinary query stream still ends at its original message boundary. A late cancellation after a fully consumed response returns without sending unnecessary Attention; poisoned connections remain rejected.
2. Decoded row, metadata, return-value, server error/info and environment-change payloads are removed from tracing. Native error codes and payload-free transport events remain.
3. The Rustls backend uses the existing `ring` provider instead of introducing a second crypto implementation. Process-installed providers remain honored; default certificate/hostname verification and TLS 1.2 remain enabled. The original manifest is preserved in Cargo.toml.orig; the normalized build manifest is changed explicitly.

4. Token decoding checkpoints its wire bytes until the complete token is decoded. A dropped pending decoder can be replayed from its tag during Attention recovery instead of treating a remaining field byte as the next token. Retention is per token, capped at 32 MiB; committed tokens over 64 KiB release the retained allocation. Oversized reads and tokens that cross a completed response boundary poison the connection. The original three-second driver recovery deadline and fail-closed behavior remain unchanged.

The added SDK tests cover partial ReturnStatus, ROW and NBCROW decoding at every field split, same-session reuse, allocation release and malformed/oversized checkpoint rejection. Run `cargo test --locked --manifest-path third_party/tiberius-0.13.0/Cargo.toml --no-default-features --features rustls,tds73,chrono --lib`. This separate SDK development graph is not added to the application workspace or application lockfile.

Real SQL Server driver contracts in `crates/drivers/mssql/tests/integration.rs` reproduce the Attention issue via row truncation, executing-query cancellation, result backpressure and consumer loss, and verify a fresh SELECT on the same session afterward. Failed synchronization still closes the wrapper session.

The real `real_sql_server_partial_token_cancellation` regression shapes only an owned loopback plaintext fixture response into two valid TDS packets, holding the remainder until Attention is sent. Four metadata-field prefixes verify cancellation and SELECT 42 reuse on the same connection. It fails against the pre-checkpoint decoder and passes with this patch. Configure `KLYNDB_TEST_MSSQL_FRAGMENTED_URL` with explicit `tls=disabled` alongside the existing disposable password for local CI; verified-TLS contracts remain separate. No server settings or database writes are needed.
