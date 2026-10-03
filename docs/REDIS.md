# Redis key workspace

Current source builds add a dedicated Redis workspace. Downloadable Preview 1 is unchanged. Actual macOS source debug-app acceptance covers native keys, types/TTL, six value types, exact reads, production review, read-only guards and a delayed real reply across a tab switch. Updated public packages and Windows/Linux acceptance remain pending.

Choose **Redis** in New connection. Use `redis://default@host:6379/0`, or an ACL username and database number. Passwords use the separate password field and optional OS keychain storage. TLS verifies the hostname and certificate by default; `rediss://` always requires TLS. Choose a custom CA or PKCS#12 client identity in the existing TLS controls. SSH uses the existing pinned-host tunnel and preserves the Redis hostname for TLS verification. Real Redis TLS/mTLS contracts are verified separately from pending Redis SSH and native desktop TLS/mTLS interaction.

For a trusted disposable plaintext server only, add `?tls=disabled`. Connection timeout remains configurable from 1–300 seconds. Each command/metadata request has a 10-second deadline. Transport errors, oversized wire responses and request timeouts close the session; reconnect explicitly. Commands are never automatically replayed.

## Browse and inspect

Open **Key explorer**, enter a Redis glob pattern such as `cache:*`, then choose **Scan keys**. No whole-keyspace load occurs on connection. **Next scan step** follows the server's original cursor. An empty step with a nonzero cursor is not the end of the scan. Concurrent changes can move or duplicate keys, and `COUNT 100` is a hint rather than a guaranteed page size. The UI shows the current step's count, not a fabricated database total. Steps above 1,000 keys are refused; narrow the pattern.

Select a key to load its native type, TTL and length. TTL is a snapshot: `-1` means no expiry, `-2` means the key disappeared. **Refresh value** starts inspection again.

| Type | Native inspection | Paging |
| --- | --- | --- |
| String | STRLEN / GETRANGE | 64 KiB byte ranges |
| Hash | HLEN / HSCAN | Native cursor, field/value pairs |
| List | LLEN / LRANGE | 100 indexed items |
| Set | SCARD / SSCAN | Native cursor |
| Sorted set | ZCARD / ZSCAN | Native cursor, member/score pairs |
| Stream | XLEN / XRANGE | 100 entries; exclusive last-ID continuation |

Text remains escaped display data. Invalid UTF-8 keys/values display original hexadecimal bytes. Integers remain decimal strings through IPC, preserving signed 64-bit values. Byte-range boundaries can split UTF-8 characters; that slice displays hexadecimal without losing bytes. Hash/collection scans may return the entire compact encoding despite COUNT; large responses fail explicitly rather than silently truncating fields.

## Native command console

Enter one **JSON argument array**, then choose **Run command** or Cmd/Ctrl+Enter in the input:

```json
["GET", "cache:key"]
```

```json
["HSET", "profile:42", "name", "Ada"]
```

JSON strings preserve whitespace, quotes and NUL characters in native arguments. The current console accepts text arguments; hexadecimal display is not automatically decoded into binary arguments. Use `TTL`, `PTTL`, `EXPIRE`, `PERSIST`, ordinary key/type commands and the supported string/hash/list/set/sorted-set/stream data commands for inspection and changes.

Rust classifies commands before execution. Read-only connections reject writes, including SET with GET. Production writes require confirmation of the captured exact argument array, checked again against the open session's pinned configuration. Restricted server ACLs remain the authoritative server-side protection. The command draft, glob pattern, completed scan step, selection, value paging and command reply stay in memory across tab switches; they are not stored in SQL history; workspace files exclude them by default. Redis tabs share a 16 MiB estimated UTF-16 cache budget. Older result payloads are cleared with a visible notice when needed; drafts and patterns remain. This is a payload-cache limit, not a measured process-RAM bound. Disconnect/reconnect clears cached server data and cursors without replaying commands. Closing or reassigning the tab clears its transient state, including its command draft. Key tab identities and saved connections restore normally after restart. Enable **Restore MongoDB and Redis drafts after restart** in Workspace settings to also restore command arguments and the authored glob pattern. This is off by default because literals are saved locally without encryption and may be sensitive. Results, keys, scan cursors, inspections and confirmations are excluded. Connect and run explicitly: a restored command never executes automatically, and production writes still require review. Turning the setting off removes saved native drafts on the next successful workspace save while keeping current text in memory; this is not a secure-erase guarantee. Draft text fields are limited to 1 MiB each and the existing workspace document to 8 MiB. A save failure is visible and prevents a normal window close until resolved.

The console currently refuses administrative/session commands, AUTH/SELECT/HELLO, KEYS/FLUSH*, transactions, scripting, blocking calls, subscriptions and unknown module commands. Cluster redirection, Sentinel, RedisJSON/modules and binary argument editing remain pending. This is a standalone RESP2 connection, not a full redis-cli terminal.

## Bounds and failures

Keys are limited to 64 KiB; patterns to 1,024 bytes. Commands are limited to 256 KiB and 1,000 arguments. Native wire responses are limited to 8 MiB; displayed responses to 4 MiB, 10,000 values and 32 nested array levels. Request smaller ranges when a response exceeds a limit. Transport failure after a write has an uncertain outcome: verify effects before retrying. A successful native write whose reply exceeds the display limit is reported as completed, without pretending it rolled back.

Redis has no SQL table inspector, relational result grid, transactions, query plans or SQL import/export controls. No automatic rollback is promised. Each operation uses the existing session lock; the UI guards close/reconnect/connection switching while a key operation is pending.

Implementation is original. Only public Redis command documentation and Beekeeper's public Redis workflow documentation were used as behavior references; no community or commercial source was copied. See the dependency notices for redis-rs and the validation log for the exact tested scope.

Scans, inspections, command replies and errors remain with their tab even when they arrive after switching away. Returning during a pending request keeps its controls guarded; completion updates the restored tab without replaying the command. Production writes still require review: if classification finishes while the tab is away and the command has not been confirmed, it is not submitted; return and run it again to review. Already confirmed commands continue normally. Late replies from a cleared session or reassigned/closed tab are discarded. Opt-in restart restoration retains drafts only; returned server data stays transient.

Actual macOS source debug-app checks restore an unsent SET argument array and glob pattern after a complete close/relaunch, with no connection, keys, cursor, reply or command execution. Disabling preserves live text, removes saved draft keys from the native SQLite workspace and restores `*` / `["PING"]` defaults on a later launch. This validates draft persistence; native Redis server workflows are now verified on macOS as described below. See [VALIDATION.md](VALIDATION.md).

### Native macOS source acceptance

The actual embedded source debug app at `6c8f33d` connects to a separate Redis 7.4.11 synthetic key prefix with a restricted user and session-only password. Test connection closes its temporary session. Native scan and inspection verify strings, exact 64-bit text, hexadecimal binary bytes, hashes, lists, sets, sorted sets, streams and TTLs. Production review Cancel leaves the reviewed key absent; Confirm stores the exact reviewed value in an independent server observation. Administrative commands and read-only writes are refused; reads still work. Disconnect clears server data and preserves the live draft.

A real GET reply held for six seconds by an owned localhost transport proxy reaches the remounted tab after leaving/returning; controls stay guarded while pending. The proxy ledger confirms one request and no replay. This validates source-app native IPC and Redis behavior on macOS, not other platforms, Redis SSH or an updated optimized public package. [Validation receipts](VALIDATION.md) and [native screenshot tour](DEMO.md).

The exact optimized macOS Preview 2 candidate now passes a disposable Redis connection, lazy prefix scan, all six native types, exact integer/binary values and TTLs. A production SET review is cancelled without creating its key, then confirmed with its exact Unicode value independently checked on the server. This loopback check explicitly disables TLS and uses session-only credentials; native TLS/mTLS/SSH, delayed replies/draft restoration/read-only and platform checks remain separate. The candidate is unpublished; see [validation evidence](VALIDATION.md#preview-2-exact-package-sql-server-clickhouse-mongodb-and-redis-acceptance--2026-10-03).
