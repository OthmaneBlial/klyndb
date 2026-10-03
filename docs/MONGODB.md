# MongoDB document workspace

Source builds add a dedicated MongoDB workspace. Preview 1 does not contain this driver. The source implementation uses the official Apache-2.0 MongoDB Rust driver **3.9.1** and BSON 3.1; it does not translate MongoDB into SQL.

## Connect

Use a single-seed URL such as `mongodb://user@localhost:27017/database?authSource=admin`, or a `mongodb+srv://` address. Enter the password separately; the existing OS Keychain controls apply. Verified TLS is the default. Use `tls=disabled` only for an explicitly trusted local server. SRV URLs require TLS.

Accepted options: `authSource`, `authMechanism` (`SCRAM-SHA-1` or `SCRAM-SHA-256`), `replicaSet`, `directConnection`, `connect_timeout` (1–300 seconds), `tls`, `sslrootcert`, and `sslidentity`. Passwords imported from supported URLs are removed before metadata is stored. Unsafe certificate-bypass, unacknowledged-write, retry and arbitrary native options are unavailable.

Custom CA files can contain PEM certificates or DER. MongoDB client identity files contain **both certificate chain and private key in PEM**, including supported encrypted PKCS#8 keys; other drivers retain their PKCS#12 identity format. Files are validated and bounded to 1 MiB, copied to private temporary files for the native client, then removed on disconnect. Certificate and hostname verification stay enabled.

The native library discovers replica-set members from a seed. Multiple explicit seed authorities, SSH transport, SOCKS proxies, Atlas deployment acceptance, enterprise/cloud authentication, and native desktop/platform acceptance remain pending. They are not advertised as validated support. Driver 3.9.1 requires MongoDB Server 4.4 or newer; tested server versions belong in VALIDATION.md.

## Browse and query

1. Connect and open **Documents**.
2. Enter a database name, or use **List databases** to list authorized databases.
3. **Load collections** when needed. Filter the loaded names locally; collections, views and time-series are identified separately.
4. Open a collection and run a JSON **Find documents** filter, with an optional sort object (`1` ascending, `-1` descending).
5. Switch to **Aggregation pipeline** for a JSON stage array. Use **Indexes** for on-demand native metadata. Select a result to inspect **JSON** or an expandable **Tree**.

Example find filter:

```json
{"active": true, "balance": {"$gte": {"$numberDecimal": "100.00"}}}
```

Example pipeline:

```json
[
  {"$match": {"active": true}},
  {"$group": {"_id": "$region", "customers": {"$sum": 1}}},
  {"$sort": {"customers": -1}}
]
```

Submitted JSON object order is preserved, including compound sort priority and nested BSON equality. Find sorts add `_id` as a tie-breaker. Pipelines preserve their authored order; add a deterministic `$sort` before paging if repeatable ordering matters. Live edits can move rows between offset pages. Each page contains at most **100 documents**, with a one-document native cursor batch. Offsets stop at one million; narrow filters for deep paging. A single returned BSON document and submitted JSON/BSON document are limited to **1 MiB**, with an **8 MiB serialized page** limit. Large documents require a projected read-only aggregation. Query/getMore work shares a 10-second page deadline; native server maxTimeMS is also set. Other requests have 10-second deadlines; timeout cleanup can take up to another 10 seconds. Timeout closes the native client and requires reconnect.

Database/collection catalogs are requested explicitly. Catalogs over 10,000 names or the reply limit return an error instead of presenting a silently incomplete list. No startup-wide schema scan occurs. Draft filters, pipeline text and document edits stay in memory by default and are not added to SQL history or saved secrets. Each tab retains its database/collection, filter or pipeline, authored sort, catalog, completed page, selection and JSON/tree mode while the app stays open. No request is replayed merely by switching tabs. Cached result payloads have a shared 16 MiB estimated UTF-16 budget for document tabs; older catalogs/pages/selections are cleared with a visible notice when needed, preserving the query draft and target namespace. This budget is not a process-RAM measurement. Disconnect/reconnect invalidates cached results and original BSON write targets; reload collection metadata before editing. Closing or reassigning a tab releases its transient state. Enable **Restore MongoDB and Redis drafts after restart** in Workspace settings to retain query text, authored sort, target database/collection, search and JSON/tree mode in the local workspace database. It is off by default: literals are saved without encryption and may be sensitive. Results, catalogs, original BSON snapshots and document-edit dialogs are never restored. Reopen and connect explicitly; queries never run automatically. Reload collection metadata before editing. Turning the setting off removes saved native drafts on the next successful workspace save while keeping current text in memory. This is not a secure-erase guarantee. Draft text fields are limited to 1 MiB each and the existing workspace document to 8 MiB; saving errors remain visible and prevent a normal window close until resolved.

## BSON stays BSON

Canonical Extended JSON crosses IPC as **text**, avoiding JavaScript integer rounding. ObjectId, signed 64-bit integers, Decimal128, dates, binary subtypes, arrays and nested documents retain their BSON tags. Plain JSON integers must fit signed 64-bit BSON; larger values require `$numberDecimal`. Plain fractional numbers are BSON doubles; use `$numberDecimal` when decimal precision matters.

Aggregation results are read-only, even if an `_id` appears in the result. Read-only pipeline operators are allowed explicitly. `$out`, `$merge`, `$where`, `$function` and `$accumulator` are rejected recursively, including nested pipelines. There is no unrestricted administration or server-JavaScript console.

## Reviewed document writes

Existing writable collections support one **insert**, **replacement**, or **delete** at a time. Views and time-series are read-only here. Every UI edit has a review step with the exact submitted document and target; production connections require confirmation in Rust against the pinned open-session configuration. Read-only connections reject writes in both core and driver. Native permissions still apply.

Replacement and deletion retain the original ordered BSON snapshot and use one atomic native predicate matching `_id` and the whole original document. Concurrent field/value/order changes reject the edit. This uses MongoDB's **native equality**, including its numeric comparison semantics; it is not byte-identical numeric-type locking. Replacement must preserve the original BSON `_id`. Upsert is disabled. Writes request majority acknowledgement and native counts; they are not a multi-document transaction or staged batch.

Automatic read/write retries are disabled. If a request fails or times out after submission, a write may already have completed: reconnect and refresh before retrying. An acknowledged write followed by a failed refresh is reported explicitly. No document content, native error payload or credential-bearing URI is returned in native failure messages.

## Validation and limits

Run `./scripts/check.sh` locally with disposable MongoDB fixture variables. The new real-server contract covers collection/index metadata, 205-document paging, typed Extended JSON, find/aggregation, stale replacements/deletes, successful edits, duplicates, read-only native ACLs, views, size limits and disconnect. Core tests cover pinned production/read-only configuration. Optional `KLYNDB_TEST_MONGODB_OBSERVATIONS_PATH` records only the disposable contract’s synthetic catalog/page/index artifacts for component checks. Separate TLS tests cover correct/wrong CA, hostname, required client identity, wrong identity, encrypted keys and wrong key passwords.

TLS fixture variables are `KLYNDB_TEST_TLS_MONGODB_URL`, `KLYNDB_TEST_MTLS_MONGODB_URL`, `KLYNDB_TEST_TLS_CERT_DIR` (containing `ca.pem`/`other-ca.pem`), `KLYNDB_TEST_MONGODB_IDENTITY_DIR` (containing `client.pem`, `wrong-client.pem`, `encrypted-client.pem`) and `KLYNDB_TEST_MONGODB_IDENTITY_PASSWORD`. Use separately owned disposable servers with matching writer credentials and required client-certificate verification.

Current validation status is recorded in [VALIDATION.md](VALIDATION.md). The macOS source debug app has real native connection, 205-document paging, tab retention, exact BSON, indexes, grouping/tree inspection, reviewed insert/replace/delete and Cancel and failed session-only reconnect checks. Broader native desktop acceptance, replica-set/SRV acceptance, cancellation controls, index editing, collection creation/drop, imports/exports, document transactions and broader platform acceptance remain unfinished.

## Provenance

Implemented independently from workflow requirements and [public Beekeeper MongoDB documentation](https://docs.beekeeperstudio.io/user_guide/connecting/mongodb/). No reference source or assets were copied. Native behavior follows the [official Rust driver](https://github.com/mongodb/mongo-rust-driver), [TLS documentation](https://www.mongodb.com/docs/drivers/rust/current/security/tls/) and [release notes](https://www.mongodb.com/docs/drivers/rust/current/reference/release-notes/). Locked dependency license texts ship in the application notice inventory.

Replies, metadata, acknowledgements and errors remain with their tab even when they arrive after switching away. Returning during a pending request keeps its controls guarded; completion updates the restored tab without replaying the request. An acknowledged write remains visible if its subsequent refresh fails. Late replies from a cleared session or reassigned/closed tab are discarded. Opt-in restart restoration retains drafts only; returned server data stays transient.
