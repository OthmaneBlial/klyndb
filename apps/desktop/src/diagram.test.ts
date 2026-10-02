import { expect, it } from "vitest";
import type { DiagramTable } from "./api";
import {
  autoLayout,
  cardHeight,
  relationPath,
  restoreDiagram,
  tableKey,
} from "./diagram";

it("lays out different-sized tables without overlap and restores only bounded layouts", () => {
  const table = (name: string, count: number): DiagramTable => ({
    table: { schema: "main", name, kind: "table" },
    columns: Array.from({ length: count }, (_, i) => ({
      name: `c${i}`,
      data_type: "INTEGER",
      primary_key: i === 0,
      nullable: false,
      default: null,
      generated: false,
    })),
    relationships: [],
  });
  const model = {
    tables: [
      table("a", 15),
      table("b", 2),
      table("c", 4),
      table("d", 1),
      table("e", 8),
    ],
  };
  const positions = autoLayout(model);
  for (let i = 0; i < model.tables.length; i++)
    for (let j = i + 1; j < model.tables.length; j++) {
      const a = model.tables[i],
        b = model.tables[j],
        p = positions[tableKey(a.table)],
        q = positions[tableKey(b.table)];
      expect(
        p.x + 280 <= q.x ||
          q.x + 280 <= p.x ||
          p.y + cardHeight(a) <= q.y ||
          q.y + cardHeight(b) <= p.y,
      ).toBe(true);
    }
  expect(tableKey({ schema: "a.b", name: "c" })).not.toBe(
    tableKey({ schema: "a", name: "b.c" }),
  );
  const layout = {
    tables: model.tables.map((t) => t.table),
    positions,
    zoom: 1,
    pan: { x: 0, y: 0 },
  };
  expect(restoreDiagram(layout)).toEqual(layout);
  expect(restoreDiagram({ ...layout, zoom: NaN })).toBeNull();
  expect(restoreDiagram({ ...layout, pan: { x: Infinity, y: 0 } })).toBeNull();
  const self = model.tables[0];
  expect(
    relationPath(self, self, "c1", "c0", { x: 0, y: 0 }, { x: 0, y: 0 }),
  ).toBe("M 280 86 C 350 86, 350 62, 280 62");
});
