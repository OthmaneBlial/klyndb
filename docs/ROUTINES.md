# Functions and procedures

Current source builds can browse PostgreSQL functions and procedures. Downloadable Preview 1 does not include this browser. Other engines' routine catalogs remain pending.

1. Connect to PostgreSQL, expand its connection and choose **Functions & procedures**. You can also choose **Browse functions & procedures** from the command palette while that connection's SQL tab is active.
2. Search by schema or routine name. Search treats `%` and `_` literally. Use **Previous**, **Next** or **Refresh** to navigate the catalog.
3. Select an overload by its argument signature. The viewer shows its language, native return type when applicable, and server-reconstructed `CREATE OR REPLACE FUNCTION` or `CREATE OR REPLACE PROCEDURE` definition.
4. Choose **Open in SQL tab** to review or edit the definition in a new tab on the same connection. Opening and inspecting never execute the routine or its definition. Run SQL explicitly through the normal query workflow when ready.

The browser loads only when opened. Catalog requests return at most 100 routines, and definitions are fetched individually after selection. Results cover the current database's user namespaces for which the session has schema USAGE; system routines and aggregates are omitted. Read-only connections can browse and inspect. Existing queries and transactions keep their session order: metadata reads wait behind active work rather than opening a separate session.

Definitions are reconstructed by PostgreSQL, rather than the literal original creation text. Dropped or inaccessible objects require a refresh. Definitions above 2 MiB are refused instead of clipped into incomplete SQL. Search is limited to 1,024 UTF-8 bytes; paging is limited to an offset of 1,000,000. Concurrent schema changes can shift pages. No catalog-wide count or large-schema performance claim is made.

Routine definitions opened in SQL tabs follow the normal local workspace persistence. Browsing alone does not add queries to history or modify schema. Native desktop interaction and Windows/Linux acceptance of the new browser remain pending; real PostgreSQL contracts and source-build evidence are recorded in [VALIDATION.md](VALIDATION.md).
