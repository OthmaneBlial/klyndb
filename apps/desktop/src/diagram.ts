import type { Diagram, DiagramTable, Table } from "./api";
export const CARD_WIDTH = 280,
  HEADER = 50,
  ROW = 24;
export interface Point {
  x: number;
  y: number;
}
export interface DiagramLayout {
  tables: Table[];
  positions: Record<string, Point>;
  zoom: number;
  pan: Point;
}
export const tableKey = (table: Pick<Table, "schema" | "name">) =>
  JSON.stringify([table.schema, table.name]);
export const cardHeight = (table: DiagramTable) =>
  HEADER + ROW * table.columns.length + 8;
export const short = (value: string, max: number) =>
  [...value].length > max ? [...value].slice(0, max - 1).join("") + "…" : value;
export const bounded = (value: number) =>
  Math.max(-100000, Math.min(100000, value));
// ponytail: grid layout avoids a graph dependency; add routed layout when crowded diagrams justify it.
export function autoLayout(model: Diagram): Record<string, Point> {
  const result: Record<string, Point> = {};
  const tables = [...model.tables].sort(
    (a, b) =>
      a.relationships.length - b.relationships.length ||
      tableKey(a.table).localeCompare(tableKey(b.table)),
  );
  const columns = Math.max(1, Math.ceil(Math.sqrt(tables.length)));
  let y = 40;
  for (let i = 0; i < tables.length; i += columns) {
    const row = tables.slice(i, i + columns);
    row.forEach((t, j) => {
      result[tableKey(t.table)] = { x: 40 + j * (CARD_WIDTH + 120), y };
    });
    y += Math.max(...row.map(cardHeight)) + 100;
  }
  return result;
}
export function relationPath(
  source: DiagramTable,
  target: DiagramTable,
  from: string,
  to: string | null,
  p: Point,
  q: Point,
) {
  const sy =
    p.y +
    HEADER +
    ROW *
      (Math.max(
        0,
        source.columns.findIndex((c) => c.name === from),
      ) +
        0.5);
  const ty =
    q.y +
    HEADER +
    ROW *
      (Math.max(
        0,
        target.columns.findIndex((c) => c.name === to),
      ) +
        0.5);
  const self = tableKey(source.table) === tableKey(target.table);
  const right = q.x >= p.x;
  const sx = p.x + (right ? CARD_WIDTH : 0),
    tx = q.x + (self || !right ? CARD_WIDTH : 0);
  const bend = right ? 70 : -70;
  return `M ${sx} ${sy} C ${sx + bend} ${sy}, ${tx + (self ? 70 : -bend)} ${ty}, ${tx} ${ty}`;
}
export function restoreDiagram(value: unknown): DiagramLayout | null {
  if (!value || typeof value !== "object") return null;
  const v = value as Partial<DiagramLayout>;
  const point = (p: unknown): p is Point =>
    !!p &&
    typeof p === "object" &&
    ["x", "y"].every(
      (k) =>
        typeof (p as Record<string, unknown>)[k] === "number" &&
        Number.isFinite((p as Record<string, unknown>)[k]) &&
        Math.abs((p as Record<string, number>)[k]) <= 100000,
    );
  if (
    !Array.isArray(v.tables) ||
    v.tables.length > 50 ||
    !v.tables.every(
      (t) =>
        t &&
        [t.schema, t.name, t.kind].every(
          (s) => typeof s === "string" && s.length <= 16384,
        ),
    ) ||
    !v.positions ||
    typeof v.positions !== "object" ||
    Array.isArray(v.positions) ||
    Object.keys(v.positions).length > 50 ||
    !Object.values(v.positions).every(point) ||
    typeof v.zoom !== "number" ||
    !Number.isFinite(v.zoom) ||
    v.zoom < 0.1 ||
    v.zoom > 3 ||
    !point(v.pan)
  )
    return null;
  return v as DiagramLayout;
}
