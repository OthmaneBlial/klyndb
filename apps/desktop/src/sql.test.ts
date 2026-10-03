import { describe, expect, it } from "vitest";
import { EditorState, type TransactionSpec } from "@codemirror/state";
import { sql, PostgreSQL, MSSQL } from "@codemirror/lang-sql";
import { ClickHouseSQL, currentStatement, replaceDocument } from "./sql";
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

it("synchronizes generated SQL without dispatching unchanged documents", () => {
  let updates = 0;
  const editor = {
    state: EditorState.create({
      doc: "SELECT * FROM items LIMIT 500 OFFSET 0;",
    }),
    dispatch: (...specs: TransactionSpec[]) => {
      editor.state = editor.state.update(...specs).state;
      updates++;
    },
  };
  const next =
    "SELECT * FROM items WHERE id >= 9500 ORDER BY id DESC LIMIT 500 OFFSET 500;";
  replaceDocument(editor, next);
  expect(editor.state.doc.toString()).toBe(next);
  replaceDocument(editor, next);
  expect(updates).toBe(1);
});

it("keeps ClickHouse escaped quotes and semicolons inside the current statement", () => {
  const doc = "SELECT 'a\\';b' AS label; SELECT 42;";
  const state = EditorState.create({
    doc,
    selection: { anchor: 12 },
    extensions: [sql({ dialect: ClickHouseSQL })],
  });
  expect(currentStatement(state).trim()).toBe("SELECT 'a\\';b' AS label;");
});

it("keeps T-SQL bracket identifiers and Unicode strings inside the current statement", () => {
  const doc = "SELECT TOP (1) N'é;雪' AS [semi;colon]; SELECT 42;";
  const state = EditorState.create({
    doc,
    selection: { anchor: 20 },
    extensions: [sql({ dialect: MSSQL })],
  });
  expect(currentStatement(state).trim()).toBe(
    "SELECT TOP (1) N'é;雪' AS [semi;colon];",
  );
});
