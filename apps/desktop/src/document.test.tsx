import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { DocumentWorkspace } from "./components/DocumentWorkspace";
import { restoreWorkspace, workspaceKind } from "./workspace";
import type { Capabilities } from "./api";
import { updateConnectTimeout, updateTls } from "./connection";

test("MongoDB has a document workspace, native capability precedence and TLS URL controls", () => {
  expect(workspaceKind(undefined, "mongodb")).toBe("document");
  expect(workspaceKind({ document_queries: false, key_value: false } as Capabilities, "mongodb")).toBe("sql");
  expect(workspaceKind({ document_queries: true, key_value: false } as Capabilities, "postgres")).toBe("document");
  const tabs = [{ id: "mongo", name: "Documents", connection: "1", sql: "", kind: "document" }];
  expect(restoreWorkspace({ tabs }).tabs[0].kind).toBe("document");
  const html = renderToStaticMarkup(<DocumentWorkspace ready={false} onBusy={() => {}} />);
  expect(html).toContain("MongoDB document workspace");
  expect(html).toContain("Aggregation pipeline");
  expect(html).toContain("Document filter");
  expect(html).not.toContain("SQL editor");
  expect(html).toContain("Drafts stay in memory");
  const address=updateTls("mongodb", "mongodb+srv://alice@cluster.example/db", "required", "/tmp/CA bundle.pem", "/tmp/client.pem");
  const url=new URL(updateConnectTimeout(address,"45"));
  expect(url.protocol).toBe("mongodb+srv:");
  expect(url.searchParams.get("sslidentity")).toBe("/tmp/client.pem");
  expect(url.searchParams.get("connect_timeout")).toBe("45");
});
