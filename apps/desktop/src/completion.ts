import {
  ifNotIn,
  type Completion,
  type CompletionContext,
  type CompletionResult,
  type CompletionSource,
} from "@codemirror/autocomplete";
import {
  schemaCompletionSource,
  type SQLDialect,
  type SQLNamespace,
} from "@codemirror/lang-sql";
import type { Table } from "./api";
import { tableKey } from "./diagram";
import { projectedCompletion } from "./projectedCompletion";

export interface CompletionSchema {
  tables: Table[];
  columns: Record<string, string[]>;
}

export function tableCompletion(
  schema: CompletionSchema,
  dialect: SQLDialect,
  load: (table: Table) => Promise<string[]>,
): CompletionSource {
  const columns = new Map(Object.entries(schema.columns));
  const pending = new Map<string, Promise<void>>();
  const escape = (name: string) => name.replaceAll(".", "\\.");
  const names = new Map<string, number>();
  for (const table of schema.tables)
    names.set(table.name, (names.get(table.name) ?? 0) + 1);
  const schemas = new Set(schema.tables.map((table) => table.schema));
  // ponytail: bounded namespaces avoid CodeMirror's quadratic name scans;
  // lookup fans out over pools, so measure before replacing its SQL resolver.
  const pools: Table[][] = [];
  const tablePools = new Map<string, number>();
  for (let i = 0; i < schema.tables.length; i += 64) {
    const pool = schema.tables.slice(i, i + 64);
    for (const table of pool) tablePools.set(tableKey(table), pools.length);
    pools.push(pool);
  }
  if (!pools.length) pools.push([]);
  function rebuild(pool: Table[]) {
    const namespace: Record<string, SQLNamespace> = Object.create(null);
    for (const table of pool) {
      // Private resolution markers never reach the menu. CodeMirror resolves
      // qualified names and aliases before we load only the requested table.
      const fields = columns.get(tableKey(table)) ?? [
        { label: "", klyndbTable: table },
      ];
      namespace[
        table.schema
          ? `${escape(table.schema)}.${escape(table.name)}`
          : escape(table.name)
      ] = fields;
      if (names.get(table.name) === 1 && !schemas.has(table.name))
        namespace[escape(table.name)] = fields;
    }
    return schemaCompletionSource({ schema: namespace, dialect });
  }
  const sources = pools.map(rebuild);
  async function completeCatalog(
    context: CompletionContext,
  ): Promise<CompletionResult | null> {
    let result: CompletionResult | null = null;
    const options = new Map<string, Completion>();
    for (const source of sources) {
      const found = await source(context);
      if (context.aborted) return null;
      if (!found) continue;
      result ??= found;
      for (const option of found.options) {
        const key =
          "klyndbTable" in option
            ? `table:${tableKey(option.klyndbTable as Table)}`
            : `label:${option.label}`;
        if (!options.has(key)) options.set(key, option);
      }
    }
    return result ? { ...result, options: [...options.values()] } : null;
  }
  async function loadColumns(table: Table) {
    const id = tableKey(table);
    if (columns.has(id)) return columns.get(id)!;
    let request = pending.get(id);
    if (!request) {
      request = load(table)
        .then((names) => {
          columns.set(id, names);
          const pool = tablePools.get(id);
          if (pool !== undefined) sources[pool] = rebuild(pools[pool]);
        })
        .finally(() => pending.delete(id));
      pending.set(id, request);
    }
    await request;
    return columns.get(id)!;
  }
  return ifNotIn(["String", "LineComment", "BlockComment"], async (context) => {
    const local = await projectedCompletion(
      context,
      dialect,
      schema.tables,
      loadColumns,
    );
    if (context.aborted) return null;
    if (local?.qualified) return local.result;
    let result = await completeCatalog(context);
    if ((!result && !local) || context.aborted) return null;
    const requested =
      result?.options.flatMap((option) =>
        "klyndbTable" in option ? [option.klyndbTable as Table] : [],
      ) ?? [];
    for (const table of requested) {
      await loadColumns(table);
      if (context.aborted) return null;
    }
    if (requested.length) result = await completeCatalog(context);
    if (local && result) {
      const labels = new Set(
        local.result.options.map((option) => option.label),
      );
      result = {
        ...result,
        options: [
          ...local.result.options,
          ...result.options.filter((option) => !labels.has(option.label)),
        ],
      };
    } else if (local) result = local.result;
    return result;
  });
}
