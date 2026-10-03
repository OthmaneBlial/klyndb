import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { RoutineDefinition } from "./components/RoutineBrowser";

test("native routine signatures and reconstructed definitions remain escaped text", () => {
  const html = renderToStaticMarkup(
    <RoutineDefinition
      routine={{
        id: "42",
        schema: "app",
        name: "<overloaded>",
        kind: "function",
        arguments: "value integer",
        returns: "integer",
        language: "sql",
      }}
      definition={
        "CREATE OR REPLACE FUNCTION app.test() RETURNS text AS $$ SELECT '<script>é</script>' $$ LANGUAGE sql;"
      }
      onOpen={() => {}}
    />,
  );
  expect(html).toContain("app.&lt;overloaded&gt;(value integer)");
  expect(html).toContain("function · sql → integer");
  expect(html).toContain("&lt;script&gt;é&lt;/script&gt;");
  expect(html).not.toContain("<script>");
  expect(html).toContain("Opening a tab does not execute it.");
  expect(html).toContain("Open in SQL tab");
});
