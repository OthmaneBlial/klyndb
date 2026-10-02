import { expect, test } from "vitest";
import { previewValue, suggestMapping } from "./import";
import type { Column, ImportOptions } from "./api";
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

test("JSON previews preserve NULL, literal null, whitespace and exact numbers", () => {
  const options: ImportOptions = {
    format: "json",
    delimiter: ",",
    trim: true,
    empty_as_null: true,
    null_value: "null",
  };
  expect(previewValue(null, options)).toBeNull();
  expect(previewValue("null", options)).toBe("null");
  expect(previewValue("", options)).toBe("");
  expect(previewValue("  value  ", options)).toBe("  value  ");
  expect(previewValue("18446744073709551615", options)).toBe(
    "18446744073709551615",
  );
  expect(previewValue("null", { ...options, format: "csv" })).toBeNull();
  expect(previewValue("   ", { ...options, format: "csv" })).toBeNull();
});
