-- Catalog estimates and native file sizes only; never scan the table or run ANALYZE.
SELECT CASE WHEN c.reltuples >= 0 THEN c.reltuples::text END,
       pg_catalog.pg_table_size(c.oid)::text,
       pg_catalog.pg_indexes_size(c.oid)::text,
       pg_catalog.pg_total_relation_size(c.oid)::text
FROM pg_catalog.pg_class c
JOIN pg_catalog.pg_namespace n ON n.oid = c.relnamespace
WHERE n.nspname = $1 AND c.relname = $2
  AND c.relkind IN ('r', 'm')
  AND pg_catalog.has_schema_privilege(n.oid, 'USAGE')
  AND (pg_catalog.has_table_privilege(c.oid, 'SELECT,INSERT,UPDATE,DELETE,TRUNCATE,REFERENCES,TRIGGER')
       OR pg_catalog.has_any_column_privilege(c.oid, 'SELECT,INSERT,UPDATE,REFERENCES'))
