import type {
  Column,
  ImportMapping,
  ImportOptions,
  ImportValueKind,
} from "./api";

export function importValueKind(type: string): ImportValueKind {
  if (/^bool(?:ean)?\b/i.test(type)) return "boolean";
  if (/^jsonb?\b/i.test(type)) return "json";
  if (
    /^(?:bytea|blob|binary|varbinary|tinyblob|mediumblob|longblob)\b/i.test(
      type,
    )
  )
    return "binary";
  if (
    /^(?:smallint|integer|int|bigint|numeric|decimal|float|real|double|mediumint|tinyint)\b/i.test(
      type,
    )
  )
    return "number";
  return "text";
}
export function suggestMapping(
  headers: string[],
  columns: Column[],
): ImportMapping[] {
  const used = new Set<string>();
  return headers.map((header) => {
    const available = columns.filter((c) => !c.generated && !used.has(c.name));
    const exact = available.find((c) => c.name === header);
    const folded = available.filter(
      (c) => c.name.toLowerCase() === header.toLowerCase(),
    );
    const column = exact ?? (folded.length === 1 ? folded[0] : undefined);
    if (column) used.add(column.name);
    return {
      column: column?.name ?? null,
      kind: column ? importValueKind(column.data_type) : "text",
    };
  });
}

export function previewValue(
  value: string | null,
  options: ImportOptions,
): string | null {
  if (value === null || options.format !== "csv") return value;
  const text = options.trim ? value.trim() : value;
  return text === options.null_value || (options.empty_as_null && text === "")
    ? null
    : text;
}
