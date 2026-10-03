import { expect, it, vi } from "vitest";
import {
  CompletionContext,
  type CompletionSource,
} from "@codemirror/autocomplete";
import { Compartment, EditorState } from "@codemirror/state";
import { ensureSyntaxTree, syntaxTree } from "@codemirror/language";
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
  let state = EditorState.create({
    doc: text.replace("|", ""),
    extensions: [sql({ dialect })],
  });
  // Headless states have no EditorView to finish the time-sliced parse.
  expect(ensureSyntaxTree(state, state.doc.length, 1000)).not.toBeNull();
  // ensureSyntaxTree advances the parser context; a transaction publishes it
  // to the immutable state tree that schemaCompletionSource actually reads.
  state = state.update({}).state;
  expect(syntaxTree(state).length).toBe(state.doc.length);
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
    await vi.waitFor(() => expect(load).toHaveBeenCalledTimes(1));
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

it("completes CTE and derived output aliases across SQL dialects without metadata reads", async () => {
  for (const dialect of [PostgreSQL, MySQL, MSSQL, SQLite, ClickHouseSQL]) {
    const load = vi.fn();
    const source = tableCompletion(
      { tables: [users], columns: {} },
      dialect,
      load,
    );
    expect(
      await labels(
        source,
        "WITH recent AS (SELECT id, COUNT(*) AS total FROM public.users GROUP BY id) SELECT r.| FROM recent r",
        dialect,
      ),
    ).toEqual(["id", "total"]);
    expect(
      await labels(
        source,
        "SELECT r.| FROM (SELECT id AS user_id, COALESCE(display_name, 'unknown') label FROM public.users) r",
        dialect,
      ),
    ).toEqual(["user_id", "label"]);
    expect(
      await labels(
        source,
        "WITH recent(user_id, total) AS (SELECT 1, 2) SELECT recent.| FROM recent",
        dialect,
      ),
    ).toEqual(["user_id", "total"]);
    expect(
      await labels(
        source,
        "WITH recent AS (SELECT 1 AS total) SELECT * FROM |",
        dialect,
      ),
    ).toContain("recent");
    expect(load).not.toHaveBeenCalled();
  }
});

it("expands scoped wildcards lazily and shares the existing metadata cache", async () => {
  const load = vi.fn(async (table: Table) =>
    table.schema === "audit" ? ["audit_id"] : ["id", "display_name"],
  );
  const source = tableCompletion(
    { tables: [users, audit], columns: {} },
    PostgreSQL,
    load,
  );
  expect(
    await labels(
      source,
      "WITH recent AS (SELECT u.* FROM public.users u), report AS (SELECT *, 42 AS total FROM recent) SELECT r.| FROM report r",
    ),
  ).toEqual(["id", "display_name", "total"]);
  expect(load).toHaveBeenCalledTimes(1);
  expect(load).toHaveBeenLastCalledWith(users);
  expect(
    await labels(source, "SELECT r.| FROM (SELECT * FROM audit.users) r"),
  ).toEqual(["audit_id"]);
  expect(load).toHaveBeenCalledTimes(2);
  expect(await labels(source, "SELECT u.| FROM public.users u")).toEqual([
    "id",
    "display_name",
  ]);
  expect(load).toHaveBeenCalledTimes(2);
});

it("keeps CTEs and nested aliases in their query scope, including shadowing and recursive names", async () => {
  const load = vi.fn(async () => ["native_id"]);
  const source = tableCompletion(
    { tables: [users], columns: {} },
    PostgreSQL,
    load,
  );
  expect(
    await labels(
      source,
      "WITH users AS (SELECT 1 AS local_id) SELECT users.| FROM users",
    ),
  ).toEqual(["local_id"]);
  expect(
    await labels(
      source,
      "SELECT x.| FROM (SELECT 1 AS outer_id) x WHERE EXISTS (SELECT 1 FROM (SELECT 2 AS inner_id) x)",
    ),
  ).toEqual(["outer_id"]);
  expect(
    await labels(
      source,
      "SELECT 1 FROM (SELECT 1 AS outer_id) x WHERE EXISTS (SELECT x.| FROM (SELECT 2 AS inner_id) x)",
    ),
  ).toEqual(["inner_id"]);
  expect(
    await labels(
      source,
      "WITH recent AS (SELECT 1 AS id) SELECT recent.id FROM recent; SELECT recent.|",
    ),
  ).toEqual([]);
  expect(
    await labels(
      source,
      "WITH RECURSIVE counter(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM counter WHERE n < 3) SELECT c.| FROM counter c",
    ),
  ).toEqual(["n"]);
  expect(
    await labels(
      source,
      "WITH recent AS (SELECT 1 AS total) SELECT r.| FROM (WITH recent AS (SELECT 2 AS inside) SELECT * FROM recent) r",
    ),
  ).toEqual(["inside"]);
  expect(load).not.toHaveBeenCalled();
});

it("quotes projected names and refuses ambiguous or unsupported expression labels", async () => {
  const load = vi.fn();
  const source = tableCompletion(
    { tables: [users, audit], columns: {} },
    PostgreSQL,
    load,
  );
  expect(
    await labels(
      source,
      'WITH "a.b" AS (SELECT 1 AS "日本語", 2 AS "a""b") SELECT r."|" FROM "a.b" r',
    ),
  ).toEqual(['"日本語"', '"a""b"']);
  expect(
    await labels(
      source,
      'WITH RECENT AS (SELECT ID AS USER_ID, 2 AS "MixedCase" FROM public.users) SELECT R.| FROM RECENT R',
    ),
  ).toEqual(["user_id", "MixedCase"]);
  expect(
    await labels(
      source,
      'WITH recent(USER_ID, "MixedCase") AS (SELECT 1, 2) SELECT recent.| FROM recent',
    ),
  ).toEqual(["user_id", "MixedCase"]);
  expect(
    await labels(
      source,
      "SELECT r.| FROM (SELECT COUNT(*), id + 1, id AS retained FROM users) r",
    ),
  ).toEqual(["retained"]);
  expect(
    await labels(
      source,
      "WITH recent AS (SELECT * FROM users) SELECT recent.| FROM recent",
    ),
  ).toEqual([]);
  for (const text of [
    "WITH r AS (SELECT 1 AS id) SELECT 'r.|'",
    "WITH r AS (SELECT 1 AS id) SELECT 1 -- r.|",
    "/* WITH r AS (SELECT 1 AS id) SELECT r.| */",
  ])
    expect(await labels(source, text)).toEqual([]);
  expect(load).not.toHaveBeenCalled();
});
