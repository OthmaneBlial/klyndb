import { expect, it, vi } from "vitest";
import {
  CompletionContext,
  type CompletionSource,
} from "@codemirror/autocomplete";
import { Compartment, EditorState } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
import { history, undoDepth } from "@codemirror/commands";
import { sql, PostgreSQL, MySQL, MSSQL, SQLite } from "@codemirror/lang-sql";
import { tableCompletion } from "./completion";
import { tableKey } from "./diagram";
import { ClickHouseSQL } from "./sql";
import type { Table } from "./api";

const users: Table = { schema: "public", name: "users", kind: "table" };
const audit: Table = { schema: "audit", name: "users", kind: "table" };
function context(text: string, dialect = PostgreSQL) {
  const pos = text.indexOf("|");
  const state = EditorState.create({
    doc: text.replace("|", ""),
    extensions: [sql({ dialect })],
  });
  // Headless states have no EditorView to finish the time-sliced parse.
  expect(ensureSyntaxTree(state, state.doc.length, 1000)).not.toBeNull();
  return new CompletionContext(state, pos, true);
}
async function labels(
  source: CompletionSource,
  text: string,
  dialect = PostgreSQL,
) {
  return (
    (await source(context(text, dialect)))?.options.map(
      (option) => option.label,
    ) ?? []
  );
}

it("loads only requested alias columns, shares in-flight reads and caches metadata", async () => {
  for (const dialect of [PostgreSQL, MySQL, MSSQL, SQLite, ClickHouseSQL]) {
    let resolve!: (columns: string[]) => void;
    const load = vi.fn(
      () =>
        new Promise<string[]>((done) => {
          resolve = done;
        }),
    );
    const source = tableCompletion(
      { tables: [users, audit], columns: {} },
      dialect,
      load,
    );
    expect(await labels(source, "SELECT * FROM |", dialect)).toContain(
      "public",
    );
    expect(load).not.toHaveBeenCalled();
    expect(await labels(source, "SELECT users.| FROM users", dialect)).toEqual(
      [],
    );
    const first = labels(source, "SELECT u.| FROM public.users AS u", dialect);
    const second = labels(source, "SELECT u.| FROM public.users u", dialect);
    await Promise.resolve();
    expect(load).toHaveBeenCalledTimes(1);
    expect(load).toHaveBeenCalledWith(users);
    resolve(["id", "display_name"]);
    expect(await first).toEqual(["id", "display_name"]);
    expect(await second).toEqual(["id", "display_name"]);
    expect(await labels(source, "SELECT public.users.|", dialect)).toEqual([
      "id",
      "display_name",
    ]);
    expect(load).toHaveBeenCalledTimes(1);
    expect(
      await labels(
        source,
        "SELECT u.id FROM public.users u; SELECT u.|",
        dialect,
      ),
    ).toEqual([]);
  }
});

it("keeps qualified same-name and dotted identifiers separate and uses opened metadata", async () => {
  const dotted = { ...users, schema: "a.b", name: "items.c" };
  const columns = {
    [tableKey(users)]: ["user_id"],
    [tableKey(audit)]: ["audit_id"],
    [tableKey(dotted)]: ["dot_id"],
  };
  const load = vi.fn();
  const source = tableCompletion(
    { tables: [users, audit, dotted], columns },
    PostgreSQL,
    load,
  );
  expect(await labels(source, "SELECT u.| FROM audit.users u")).toEqual([
    "audit_id",
  ]);
  expect(await labels(source, "SELECT u.| FROM public.users u")).toEqual([
    "user_id",
  ]);
  expect(await labels(source, 'SELECT d.| FROM "a.b"."items.c" d')).toEqual([
    "dot_id",
  ]);
  expect(await labels(source, 'SELECT d."|" FROM "a.b"."items.c" d')).toEqual([
    '"dot_id"',
  ]);
  expect(load).not.toHaveBeenCalled();
});

it("completes unique unqualified tables, ignores comments/strings and retries failed reads", async () => {
  const load = vi
    .fn()
    .mockRejectedValueOnce(new Error("metadata unavailable"))
    .mockResolvedValue(["id"]);
  const source = tableCompletion(
    { tables: [users], columns: {} },
    PostgreSQL,
    load,
  );
  for (const text of ["SELECT 'users.|'", "-- users.|", "/* users.| */"])
    expect(await labels(source, text)).toEqual([]);
  expect(load).not.toHaveBeenCalled();
  await expect(labels(source, "SELECT u.| FROM users u")).rejects.toThrow(
    "metadata unavailable",
  );
  expect(await labels(source, "SELECT u.| FROM users u")).toEqual(["id"]);
  expect(load).toHaveBeenCalledTimes(2);
});

it("reconfigures SQL metadata without losing document, selection or undo history", () => {
  const language = new Compartment();
  let state = EditorState.create({
    doc: "SELECT 1",
    extensions: [history(), language.of(sql({ dialect: PostgreSQL }))],
  });
  state = state.update({
    changes: { from: 7, to: 8, insert: "42" },
    selection: { anchor: 9 },
  }).state;
  expect(undoDepth(state)).toBe(1);
  state = state.update({
    effects: language.reconfigure(
      sql({ dialect: PostgreSQL, schema: { users: ["id"] } }),
    ),
  }).state;
  expect(state.doc.toString()).toBe("SELECT 42");
  expect(state.selection.main.head).toBe(9);
  expect(undoDepth(state)).toBe(1);
});
