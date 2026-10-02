import { expect, test } from "vitest";
import { suggestMapping } from "./import";
import type { Column } from "./api";
test("mapping suggestions avoid generated, duplicate and ambiguous destinations", () => {
  const column = (
    name: string,
    data_type = "TEXT",
    generated = false,
  ): Column => ({
    name,
    data_type,
    generated,
    nullable: true,
    primary_key: false,
    default: null,
  });
  const mapping = suggestMapping(
    ["ID", "id", "computed", "NAME", "payload", "missing"],
    [
      column("id", "BIGINT UNSIGNED"),
      column("computed", "BIGINT", true),
      column("Name"),
      column("name"),
      column("payload", "BYTEA"),
    ],
  );
  expect(mapping).toEqual([
    { column: "id", kind: "number" },
    { column: null, kind: "text" },
    { column: null, kind: "text" },
    { column: null, kind: "text" },
    { column: "payload", kind: "binary" },
    { column: null, kind: "text" },
  ]);
});
