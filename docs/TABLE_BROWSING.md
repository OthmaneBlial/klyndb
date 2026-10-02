# Table browsing

Open a table or view from a connected database’s explorer. The **Database** controls run filters, sorting and pagination on SQLite, PostgreSQL, MySQL or MariaDB. The generated SELECT appears in the editor and uses the normal query timeout, cancellation, history and transaction flow.

Choose a sort column and direction, add column filters, then select **Apply to database**. Filters match all conditions (AND). Operators include equality/inequality, ordered comparisons, Contains, LIKE, IS NULL and IS NOT NULL. Contains treats %, _ and ! literally; LIKE follows native database pattern rules. The database applies its own type conversion and collation. Invalid comparisons return a normal query error.

Database pages contain 100, 250 or 500 rows. Next/previous issues a fresh LIMIT/OFFSET query. There is no expensive automatic COUNT(*) and no invented total page count. A full last page can enable Next once more, yielding an empty page; Previous returns to the data. Primary keys provide default ordering and break explicit sort ties. Without a primary key, ties can move between pages. Concurrent writes can shift offsets even with a key; paging does not promise a frozen database snapshot. Deep offsets may be expensive.

The grid’s quick text filter and column-header sorting still apply only to the fetched page. For arbitrary SQL tabs, the existing result-spool pages and configured row limit remain available. Changing generated SQL switches to a custom query when executed; applying the Database controls returns to table browsing.

Staged edits must be applied or discarded before changing database pages or filters. Successful edits/imports refresh the current filtered page. Optimistic row checks, production confirmation, generated-column guards and read-only restrictions remain enforced.

Rust builds a single SELECT from native table metadata and enum operators. It validates names and request bounds, quotes identifiers using the driver, and serializes filter values with native literal rules. PostgreSQL uses explicit escape strings; MySQL/MariaDB uses UTF-8 hex conversion rather than interpreting frontend backslash escapes. No raw WHERE SQL is accepted by this API. At most 20 filters, 8 sort columns, 16 KiB per filter value and a one-billion-row offset are accepted. The current interface exposes one explicit sort column plus automatic key tie breakers.

The real-engine contract in crates/core/tests/table_browse.rs verifies pages, filtering beyond the first page, sort ties, NULLs, literal Contains and inert SQL-looking text on disposable databases. Native macOS browse validation is recorded in VALIDATION.md; Windows/Linux desktop checks remain pending.
