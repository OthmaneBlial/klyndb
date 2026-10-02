import { describe, expect, it } from "vitest";
import { EditorState } from "@codemirror/state";
import { sql, PostgreSQL } from "@codemirror/lang-sql";
import { currentStatement } from "./sql";
describe("execute current statement", () => {
  it("uses the SQL parser for semicolons inside strings and dollar quotes", () => {
    const doc = "SELECT ';'; SELECT $$a;b$$; SELECT 3;";
    const state = EditorState.create({
      doc,
      selection: { anchor: 22 },
      extensions: [sql({ dialect: PostgreSQL })],
    });
    expect(currentStatement(state).trim()).toBe("SELECT $$a;b$$;");
  });
  it("runs only the final statement with the cursor at the end", () => {
    for (const doc of ["SELECT 1; SELECT 2;", "SELECT 1; SELECT 2; \n"]) {
      const state = EditorState.create({
        doc,
        selection: { anchor: doc.length },
        extensions: [sql({ dialect: PostgreSQL })],
      });
      expect(currentStatement(state).trim()).toBe("SELECT 2;");
    }
  });
  it("does not execute a whole file when statement parsing is unavailable", () => {
    const state = EditorState.create({
      doc: "SELECT 1; DELETE FROM important_table;",
    });
    expect(currentStatement(state)).toBe("");
  });
  it("executes the selected text", () => {
    const state = EditorState.create({
      doc: "SELECT 1; SELECT 2;",
      selection: { anchor: 10, head: 18 },
      extensions: [sql()],
    });
    expect(currentStatement(state)).toBe("SELECT 2");
  });
});
