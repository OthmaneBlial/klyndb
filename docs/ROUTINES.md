# Functions and procedures

Current source builds browse PostgreSQL, MySQL, MariaDB and SQL Server functions and procedures. Downloadable Preview 1 does not include this browser.

1. Expand a connected database and choose **Functions & procedures**, or choose **Browse functions & procedures** from the command palette while its SQL tab is active.
2. Search by schema or routine name. Search treats wildcard characters literally. Use **Previous**, **Next** or **Refresh** to navigate 100 routines at a time.
3. Select a signature to inspect the native parameter and return types, language and server-provided definition. PostgreSQL overloads remain separate; MySQL/MariaDB functions and procedures sharing a name remain distinct.
4. Choose **Open in SQL tab** to review or edit the definition in a new tab on the same connection. Opening and inspecting never execute the routine or its definition.

The browser loads only when opened; definitions load individually after selection. Read-only connections can browse and inspect. Metadata reads use the existing session and wait behind active work. The browser does not open another session or invoke routines, including procedures that write data.

| Engine | Catalog and signatures | Definition |
| --- | --- | --- |
| PostgreSQL | Current database's accessible user namespaces; native identity arguments, return type and language. System routines and aggregates are excluded. | `pg_get_functiondef` reconstructs CREATE OR REPLACE SQL; it is not the original creation text. |
| MySQL / MariaDB | Visible user databases; native ordered IN/OUT/INOUT parameters, full `DTD_IDENTIFIER` types and function return types. System databases are excluded. Parameter reads cover only the current page, without GROUP_CONCAT truncation. | Native SHOW CREATE FUNCTION/PROCEDURE text, including its DEFINER. Unavailable source requires suitable server permissions. |
| SQL Server | Current database's visible user Transact-SQL procedures, scalar functions and inline/multi-statement table-valued functions. Native parameters preserve lengths, precision, schema-qualified alias/table types, OUTPUT and READONLY. CLR routines are excluded. | Original `sys.sql_modules.definition` text, when visible and unencrypted. Renaming a routine can leave its stored definition referring to its old name. |

Definitions are copied unchanged. MySQL/MariaDB SHOW CREATE may use an unqualified routine name; check the target database, schema, DEFINER and permissions before executing edited SQL. Opening a definition is an inspection workflow, not an automatic routine migration: normal SQL validation can refuse unsupported compound-body syntax.

Dropped, inaccessible or encrypted objects show an explicit error rather than fabricated source. Definitions above 2 MiB are refused instead of clipped. Search is limited to 1,024 UTF-8 bytes; paging is limited to an offset of 1,000,000. MySQL/MariaDB parameter pages above 50,000 entries are refused; SQL Server uses the existing bounded metadata transport. Native collations determine search case sensitivity. Concurrent schema changes can shift pages or signatures. No catalog-wide count or large-schema performance claim is made.

Definitions opened in SQL tabs follow normal local workspace persistence. Browsing alone does not add queries to history or modify schema. Native desktop interaction and Windows/Linux acceptance remain pending; real-server and source-build evidence is recorded in [VALIDATION.md](VALIDATION.md).

Native behavior references: [MySQL PARAMETERS](https://dev.mysql.com/doc/refman/8.4/en/information-schema-parameters-table.html), [MySQL SHOW CREATE](https://dev.mysql.com/doc/refman/8.4/en/show-create-procedure.html), [MariaDB SHOW CREATE](https://mariadb.com/docs/server/reference/sql-statements/administrative-sql-statements/show/show-create-procedure), [SQL Server parameters](https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/sys-parameters-transact-sql) and [SQL Server modules](https://learn.microsoft.com/en-us/sql/relational-databases/system-catalog-views/sys-sql-modules-transact-sql).
