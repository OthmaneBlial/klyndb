import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import {
  clearDocumentResults,
  DocumentWorkspace,
  type DocumentWorkspaceState,
} from "./components/DocumentWorkspace";
import {
  clearKeyResults,
  KeyValueWorkspace,
  type KeyWorkspaceState,
} from "./components/KeyValueWorkspace";
import { TransientWorkspaceCache } from "./transientWorkspace";
import type { Connection } from "./api";

const connection = {
  id: "fixture",
  name: "Disposable",
  environment: "production",
  read_only: false,
} as Connection;
const collection = { schema: "fixture", name: "customers", kind: "collection" };
const record = {
  json: '{"integer":{"$numberLong":"9223372036854775807"}}',
  snapshot: "original-bson",
};
const document: DocumentWorkspaceState = {
  kind: "document",
  database: "fixture",
  databases: ["fixture"],
  collections: [collection],
  collection,
  search: "cust",
  text: '{"active":true}',
  sort: '{"z":1,"a":-1}',
  aggregate: false,
  page: { documents: [record], has_more: true },
  applied: {
    database: "fixture",
    collection: "customers",
    text: "{}",
    sort: "{}",
    aggregate: false,
    offset: 100,
  },
  selected: record,
  indexes: null,
  tree: false,
  error: "",
  message: "",
};
const key: KeyWorkspaceState = {
  kind: "key_value",
  pattern: "cache:*",
  applied: "cache:*",
  page: {
    cursor: "42",
    keys: [
      {
        key: { kind: "text", value: "cache:one" },
        data_type: "string",
        ttl_ms: "-1",
      },
    ],
  },
  selected: { kind: "text", value: "cache:one" },
  inspection: null,
  positions: ["0"],
  reply: {
    kind: "cell",
    value: { kind: "number", value: "9223372036854775807" },
  },
  error: "",
  message: "",
};

test("inactive native results are evicted by recency, preserving drafts and namespaces", () => {
  const cache = new TransientWorkspaceCache<DocumentWorkspaceState>(100);
  cache.remember("first", document, 40, clearDocumentResults);
  cache.remember(
    "second",
    { ...document, text: "[]", aggregate: true },
    40,
    clearDocumentResults,
  );
  expect(cache.restore("first")?.page).toBe(document.page);
  cache.remember("third", document, 40, clearDocumentResults);
  expect(cache.restore("first")?.page).toBe(document.page);
  const evicted = cache.restore("second")!;
  expect(evicted.page).toBeNull();
  expect(evicted.selected).toBeNull();
  expect(evicted.collections).toEqual([]);
  expect(evicted.text).toBe("[]");
  expect(evicted.aggregate).toBe(true);
  expect(evicted.collection).toBe(collection);
  expect(evicted.message).toContain("limit memory");
  cache.forget("first");
  expect(cache.restore("first")).toBeUndefined();
});

test("session invalidation removes original BSON write targets without clearing other tabs", () => {
  const cache = new TransientWorkspaceCache<DocumentWorkspaceState>();
  cache.remember("mongo", document, 40, clearDocumentResults);
  cache.remember("other", document, 40, clearDocumentResults);
  cache.invalidate("mongo");
  const restored = cache.restore("mongo")!;
  expect(restored.selected).toBeNull();
  expect(restored.page).toBeNull();
  expect(restored.applied).toBeNull();
  expect(restored.text).toBe(document.text);
  expect(restored.sort).toBe(document.sort);
  expect(cache.restore("other")?.selected?.snapshot).toBe("original-bson");
  const html = renderToStaticMarkup(
    <DocumentWorkspace
      connection={connection}
      ready
      initialState={restored}
      onBusy={() => {}}
    />,
  );
  expect(html).toContain("connection session changed");
  expect(html).toMatch(
    /<button disabled=""[^>]*>.*?Insert document<\/button>/s,
  );
  expect(html).not.toContain("Edit document");
});

test("MongoDB restores exact results, authored query and page position; pending IPC stays guarded", () => {
  const html = renderToStaticMarkup(
    <DocumentWorkspace
      connection={connection}
      ready
      initialState={document}
      onBusy={() => {}}
    />,
  );
  expect(html).toContain("9223372036854775807");
  expect(html).toContain("Offset 100");
  expect(html).toContain("{&quot;z&quot;:1,&quot;a&quot;:-1}");
  expect(html).toContain("Document filter");
  expect(html).toContain(
    'aria-label="Document sort" spellCheck="false" autoCorrect="off" autoCapitalize="off"',
  );
  const blocked = renderToStaticMarkup(
    <DocumentWorkspace
      connection={connection}
      ready
      blocked
      initialState={document}
      onBusy={() => {}}
    />,
  );
  expect(blocked).toMatch(/<button class="primary" disabled=""[^>]*>.*?Run/s);
});

test("Redis restores scan cursor and exact reply, then clears server data on session change", () => {
  const cache = new TransientWorkspaceCache<KeyWorkspaceState>();
  cache.remember("redis", key, 40, clearKeyResults);
  const html = renderToStaticMarkup(
    <KeyValueWorkspace
      connection={connection}
      ready
      draft='["GET","cache:one"]'
      onDraft={() => {}}
      onBusy={() => {}}
      initialState={cache.restore("redis")}
    />,
  );
  expect(html).toContain("Cursor 42");
  expect(html).toContain("9223372036854775807");
  expect(html).toContain("cache:one");
  cache.invalidate("redis");
  expect(cache.restore("redis")?.pattern).toBe("cache:*");
  expect(cache.restore("redis")?.reply).toBeNull();
  expect(cache.restore("redis")?.positions).toEqual([]);
  expect(cache.restore("redis")?.selected).toBeNull();
});
