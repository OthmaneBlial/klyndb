# Find and reuse SQL queries

Current source builds add searchable **Saved queries** and **Query history**. Downloadable Preview 2 predates these controls; it retains the earlier query library.

Open either library from the sidebar or command palette. Search is literal and case-insensitive: it matches SQL and connection labels, plus saved names or history errors. Use **Connection** to filter by the original profile. Labels include the profile name, engine and environment; same-named profiles remain distinct by ID.

## Saved queries

Save the current editor text with Cmd/Ctrl+S or the Save query control. Name it, then reopen **Saved queries** to find it. Starred entries sort first; **Favorites only** hides unstarred entries. Stars continue to use the existing local saved-query document and survive restart. Deletion retains the existing confirmation.

## Query history

History contains the latest 500 SQL executions retained on this machine, newest first across all connections. **Failed executions only** filters unsuccessful runs; their native error text appears beside SQL, execution time and elapsed milliseconds. Searching a connection filters this retained set; it does not fetch older server history.

Choose the bookmark beside a history entry to open its exact SQL in a new tab and name a saved copy. The copy retains the original connection, including when another connection was active before opening history. Saving it does not execute it.

## Open safely

Opening either kind of entry creates a new tab with unchanged SQL and its original connection. It does not connect automatically or replay a query. An unavailable profile is labeled **Unavailable connection**; its SQL remains recoverable, with execution disabled until you intentionally choose and connect a profile. At the 100-tab limit, the library remains open when a new tab cannot be created.

Queries and history use the existing local SQLite application state. SQL is stored as text, separate from OS keychain credentials. Search/filter state is temporary and resets when closing the library. No new backend API, database migration or dependency was added. Browser production-App checks use explicitly simulated IPC; native desktop and platform evidence is tracked in [VALIDATION.md](VALIDATION.md).
