import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { DocumentWorkspace } from "./components/DocumentWorkspace";
import { KeyValueWorkspace } from "./components/KeyValueWorkspace";
import { SettingsDialog } from "./components/SettingsDialog";
import {
  defaults,
  restoreWorkspace,
  serializeWorkspace,
  withNativeDraft,
  type DocumentDraft,
  type KeyDraft,
  type Tab,
} from "./workspace";

const document: DocumentDraft = {
  kind: "document",
  database: "disposable",
  collection: "customers",
  search: "cust",
  text: '[{"$match":{"n":{"$numberLong":"9223372036854775807"}}}]',
  sort: '{"n":-1}',
  aggregate: true,
  tree: true,
};
const key: KeyDraft = {
  kind: "key_value",
  pattern: "cache:*",
  command: '["SET","cache:one","<script>é</script>"]',
};
const tabs: Tab[] = [
  {
    id: "mongo",
    name: "Documents",
    connection: "m",
    sql: "",
    kind: "document",
    draft: document,
  },
  {
    id: "redis",
    name: "Keys",
    connection: "r",
    sql: "",
    kind: "key_value",
    draft: key,
  },
  { id: "sql", name: "SQL", connection: "s", sql: "SELECT 1;" },
];
const enabled = { ...defaults, restoreNativeDrafts: true };

test("old and privacy-disabled workspaces restore tabs without native drafts", () => {
  const old = restoreWorkspace({ version: 1, tabs, active: "redis" });
  expect(old.preferences.restoreNativeDrafts).toBe(false);
  expect(old.tabs.every((tab) => tab.draft === undefined)).toBe(true);
  expect(old.active).toBe("redis");
  const saved = serializeWorkspace(tabs, "mongo", defaults);
  expect(JSON.stringify(saved)).not.toContain("9223372036854775807");
  expect(JSON.stringify(saved)).not.toContain("cache:one");
  expect(saved.tabs[2].sql).toBe("SELECT 1;");
});

test("opt-in roundtrip whitelists drafts, rejects wrong shapes, and disabling removes saved copies", () => {
  const extra = {
    ...tabs[0],
    password: "never-store",
    draft: {
      ...document,
      page: "server-result",
      snapshot: "original-bson",
      edit: "unsent-document-edit",
    },
  };
  const saved = serializeWorkspace([extra, tabs[1], tabs[2]], "redis", enabled);
  const json = JSON.stringify(saved);
  for (const forbidden of [
    "never-store",
    "server-result",
    "original-bson",
    "unsent-document-edit",
  ])
    expect(json).not.toContain(forbidden);
  const restored = restoreWorkspace(JSON.parse(json));
  expect(restored.tabs[0].draft).toEqual(document);
  expect(restored.tabs[1].draft).toEqual(key);
  expect(restored.active).toBe("redis");
  const cleared = serializeWorkspace(restored.tabs, restored.active, defaults);
  expect(restoreWorkspace(cleared).tabs.every((tab) => !tab.draft)).toBe(true);
  expect(restored.tabs[0].draft).toEqual(document); // Unsent in-memory text is kept.
  expect(() =>
    serializeWorkspace(
      [
        {
          ...tabs[0],
          draft: { ...document, text: "x".repeat(1024 * 1024 + 1) },
        },
      ],
      "mongo",
      enabled,
    ),
  ).toThrow("turn off draft restoration");
  for (const malformed of [
    key,
    { ...document, aggregate: "true" },
    { ...document, text: "x".repeat(1024 * 1024 + 1) },
  ]) {
    expect(
      restoreWorkspace({ ...saved, tabs: [{ ...tabs[0], draft: malformed }] })
        .tabs[0].draft,
    ).toBeUndefined();
  }
  expect(withNativeDraft(tabs[0], document)).toBe(tabs[0]);
  expect(withNativeDraft(tabs[0], key)).toBe(tabs[0]);
  expect(
    withNativeDraft(tabs[1], { ...key, command: '["PING"]' }).draft,
  ).toEqual({ ...key, command: '["PING"]' });
});

test("restored MongoDB draft has its namespace and exact text but no catalog or writable result", () => {
  const html = renderToStaticMarkup(
    <DocumentWorkspace ready initialDraft={document} onBusy={() => {}} />,
  );
  expect(html).toContain("customers");
  expect(html).toContain("disposable");
  expect(html).toContain("9223372036854775807");
  expect(html).toContain("Aggregation pipeline");
  expect(html).toContain("0 collections");
  expect(html).toMatch(
    /<button disabled=""[^>]*>.*?Insert document<\/button>/s,
  );
  expect(html).not.toContain("Edit document");
  expect(html).not.toContain("Offset 100");
});

test("restored Redis draft and privacy setting render as escaped unsent text", () => {
  const html = renderToStaticMarkup(
    <KeyValueWorkspace
      ready={false}
      initialDraft={key}
      draft={key.command}
      onDraft={() => {}}
      onBusy={() => {}}
    />,
  );
  expect(html).toContain("cache:*");
  expect(html).toContain("&lt;script&gt;é&lt;/script&gt;");
  expect(html).not.toContain("<script>");
  expect(html).not.toContain("9223372036854775807");
  const settings = renderToStaticMarkup(
    <SettingsDialog
      preferences={defaults}
      setPreferences={() => {}}
      onClose={() => {}}
    />,
  );
  expect(settings).toContain("Restore MongoDB and Redis drafts after restart");
  expect(settings).toContain("without encryption");
  expect(settings).toContain("never run automatically");
  expect(settings).not.toContain('checked=""');
});
