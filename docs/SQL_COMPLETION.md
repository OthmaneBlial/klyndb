# SQL column completion

Current source builds complete column names from CTEs and derived SELECT tables. Downloadable Preview 2 predates this addition. Type a relation or alias followed by a period, or use Ctrl+Space to open suggestions.

```sql
WITH recent AS (
  SELECT id AS user_id, display_name AS label
  FROM public.users
)
SELECT r.
FROM recent AS r;
```

The editor suggests `user_id` and `label`. Explicit output aliases, direct column references and explicit CTE column lists are available without a metadata request. Derived-table aliases work too:

```sql
SELECT r.
FROM (
  SELECT id AS user_id, COUNT(*) AS total
  FROM public.users
  GROUP BY id
) AS r;
```

For `SELECT *` or `alias.*`, the editor resolves the original table or earlier CTE and loads only the needed table's column metadata. These reads reuse the existing cache and in-flight requests. Same-named catalog tables still need their schema; an ambiguous unqualified table supplies no inferred wildcard columns. CTEs can shadow catalog names, and nested query aliases stay in their own scope. Recursive CTE column lists provide names without following recursive branches.

Choosing a suggestion quotes identifiers for the active SQL dialect when required, including Unicode and escaped quote characters. PostgreSQL unquoted names fold to lowercase; explicitly quoted names retain their case. Refreshing schema metadata preserves editor SQL, selection and undo history. Completion never connects, runs a query or asks the server to describe/execute a SELECT.

This is local output-name inference, not a server semantic analyzer. Use explicit aliases for computed expressions. Unaliased function/expression result names, table-valued functions, PIVOT, vendor-specific result-shape rules, recursive wildcard expansion and correlated outer-alias lookup remain outside this slice. Inference uses the existing editor syntax tree and is bounded to a 64 KiB statement and 32 query scopes; larger statements keep existing catalog completion. Suggestions are advisory and do not validate SQL or permissions.

Focused checks cover PostgreSQL, MySQL, SQL Server, SQLite and ClickHouse editor dialects. Actual Chrome exercises the production editor using column metadata from disposable SQLite fixtures; native Tauri IPC, native completion interaction and Windows/Linux acceptance remain separate. See [validation evidence](VALIDATION.md).
