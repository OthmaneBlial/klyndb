import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { ResultPanel, type Inspector } from "./components/ResultPanel";

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
