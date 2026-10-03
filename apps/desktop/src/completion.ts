import { ifNotIn, type CompletionSource } from "@codemirror/autocomplete";
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
  let source: CompletionSource;
  function rebuild() {
    const namespace: Record<string, SQLNamespace> = Object.create(null);
    for (const table of schema.tables) {
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
    source = schemaCompletionSource({ schema: namespace, dialect });
  }
  rebuild();
  async function loadColumns(table: Table) {
    const id = tableKey(table);
    if (columns.has(id)) return columns.get(id)!;
    let request = pending.get(id);
    if (!request) {
      request = load(table)
        .then((names) => {
          columns.set(id, names);
          rebuild();
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
    let result = await source(context);
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
    if (!result || context.aborted) return null;
    const requested = result.options.flatMap((option) =>
      "klyndbTable" in option ? [option.klyndbTable as Table] : [],
    );
    for (const table of requested) {
      await loadColumns(table);
      if (context.aborted) return null;
    }
    if (requested.length) result = await source(context);
    return result;
  });
}
