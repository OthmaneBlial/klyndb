import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { ResultPanel, type Inspector } from "./components/ResultPanel";
import type { QueryStatus } from "./api";

test("structure renders native constraints, trigger state and DDL fallback as text", () => {
  const inspector: Inspector = {
    table: { schema: "main", name: "items", kind: "table" },
    query: "SELECT * FROM items",
    browse: { filters: [], sort: [], limit: 500, offset: 0 },
    info: {
      editable: true,
      columns: [],
      indexes: [],
      foreign_keys: [],
      ddl: "CREATE TABLE items (id INTEGER CHECK(id > 0))",
      constraints: [
        { name: "positive", kind: "CHECK", definition: "CHECK(id > 0)" },
      ],
      triggers: [
        {
          name: "<insert>",
          definition: "INSERT INTO audit VALUES(NEW.id)",
          state: "Disabled",
        },
      ],
    },
  };
  const render = () =>
    renderToStaticMarkup(
      <ResultPanel
        inspector={inspector}
        view="structure"
        set={0}
        busy={false}
        onSelectSet={() => {}}
        onView={() => {}}
        onError={() => {}}
        onExport={() => {}}
      />,
    );
  let html = render();
  expect(html).toContain("positive");
  expect(html).toContain("CHECK(id &gt; 0)");
  expect(html).toContain("&lt;insert&gt;");
  expect(html).toContain("Disabled");
  expect(html).toContain("INSERT INTO audit VALUES(NEW.id)");
  inspector.info.constraints = null;
  inspector.info.triggers = [];
  html = render();
  expect(html).toContain(
    "Constraint definitions are shown in the table DDL below.",
  );
  expect(html).toContain("CREATE TABLE items");
  expect(html).toContain("No user triggers visible.");
});

test("result error navigation is explicit and only offered with an authoritative source", () => {
  const status: QueryStatus = {
    id: "failure",
    connection_id: "fixture",
    done: true,
    error: "column <missing> does not exist",
    error_offset: 7,
    sets: [],
    elapsed_ms: 1,
    transaction: "idle",
    plan_format: null,
    plan_analyze: false,
  };
  for (const view of ["results", "messages"] as const) {
    const render = (locate?: () => void) =>
      renderToStaticMarkup(
        <ResultPanel
          status={status}
          set={0}
          view={view}
          busy={false}
          onView={() => {}}
          onSelectSet={() => {}}
          onError={() => {}}
          onExport={() => {}}
          onLocateError={locate}
        />,
      );
    expect(render(() => {})).toContain("Go to SQL error");
    expect(render()).not.toContain("Go to SQL error");
    expect(render()).toContain("&lt;missing&gt;");
  }
});
