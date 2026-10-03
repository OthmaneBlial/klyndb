import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import type { History } from "./api";
import { Modal } from "./components/Modal";
import {
  filterSavedQueries,
  filterQueryHistory,
  QueryHistory,
  SavedQueries,
  type SavedQuery,
} from "./components/QueryLibrary";

const connections = [
  {
    id: "prod",
    name: "Orders",
    engine: "postgres" as const,
    environment: "production" as const,
  },
  {
    id: "dev",
    name: "Orders",
    engine: "postgres" as const,
    environment: "development" as const,
  },
];
const saved: SavedQuery[] = [
  {
    id: "one",
    connection: "prod",
    name: "Revenue é",
    sql: "SELECT '日本語' AS label;",
    favorite: false,
  },
  {
    id: "two",
    connection: "dev",
    name: "Revenue é",
    sql: "SELECT '<script>' AS label;",
    favorite: true,
  },
  {
    id: "three",
    connection: "removed",
    name: "Old report",
    sql: "SELECT 9223372036854775807;",
    favorite: true,
  },
];
const history: History[] = [
  {
    id: 3,
    connection_id: "prod",
    sql: saved[0].sql,
    created_at: "2026-10-03 12:00:00",
    elapsed_ms: 2,
    error: "relation <missing> does not exist",
  },
  {
    id: 2,
    connection_id: "dev",
    sql: saved[1].sql,
    created_at: "2026-10-03 11:00:00",
    elapsed_ms: 1,
    error: null,
  },
  {
    id: 1,
    connection_id: "removed",
    sql: saved[2].sql,
    created_at: "2026-10-03 10:00:00",
    elapsed_ms: 1,
    error: null,
  },
];

test("query filters preserve original SQL, connection identity, favorites and recent order", () => {
  expect(
    filterSavedQueries(saved, connections, "revenue É", "prod", false),
  ).toEqual([saved[0]]);
  expect(filterSavedQueries(saved, connections, "日本語", "", false)).toEqual([
    saved[0],
  ]);
  expect(filterSavedQueries(saved, connections, "", "", true)).toEqual([
    saved[1],
    saved[2],
  ]);
  expect(
    filterSavedQueries(saved, connections, "production", "", false),
  ).toEqual([saved[0]]);
  expect(filterSavedQueries(saved, connections, "%", "", false)).toEqual([]);
  expect(
    filterSavedQueries(
      saved,
      connections,
      "9223372036854775807",
      "removed",
      false,
    ),
  ).toEqual([saved[2]]);
  expect(saved.map((q) => q.id)).toEqual(["one", "two", "three"]);
  expect(
    filterQueryHistory(history, connections, "<missing>", "prod", true),
  ).toEqual([history[0]]);
  expect(filterQueryHistory(history, connections, "", "dev", true)).toEqual([]);
  expect(filterQueryHistory(history, connections, "", "", false)).toEqual(
    history,
  );
});

test("query libraries escape SQL and errors, identify unavailable connections and never invoke actions during rendering", () => {
  let dispatches = 0;
  const action = () => {
    dispatches++;
  };
  const savedHtml = renderToStaticMarkup(
    <Modal title="Saved queries" onClose={action}>
      <SavedQueries
        queries={saved}
        connections={connections}
        onOpen={action}
        onFavorite={action}
        onDelete={action}
      />
    </Modal>,
  );
  const historyHtml = renderToStaticMarkup(
    <QueryHistory
      history={history}
      connections={connections}
      onOpen={action}
      onSave={action}
    />,
  );
  expect(savedHtml).toContain("&lt;script&gt;");
  expect(historyHtml).toContain("&lt;missing&gt;");
  expect(savedHtml).toContain("Orders · postgres · production");
  expect(historyHtml).toContain("Unavailable connection · removed");
  expect(savedHtml).toContain('aria-pressed="true"');
  expect(historyHtml).toContain('aria-label="Save history query 3"');
  expect(historyHtml).toContain("Latest 500 stored locally");
  expect(savedHtml).toContain("Opening a tab does not execute it.");
  const labelledBy = savedHtml.match(/aria-labelledby="([^"]+)"/)?.[1];
  expect(labelledBy).toBeTruthy();
  expect(savedHtml).toContain(`<h2 id="${labelledBy}">Saved queries</h2>`);
  expect(dispatches).toBe(0);
});
