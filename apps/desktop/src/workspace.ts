import {
  defaultConfirmations,
  restoreConfirmations,
  type Confirmations,
} from "./confirmations";
import type { Capabilities } from "./api";
import {
  defaultShortcuts,
  restoreShortcuts,
  type Shortcuts,
} from "./shortcuts";
export interface DocumentDraft {
  kind: "document";
  database: string;
  collection: string;
  search: string;
  text: string;
  sort: string;
  aggregate: boolean;
  tree: boolean;
}
export interface KeyDraft {
  kind: "key_value";
  pattern: string;
  command: string;
}
export type NativeDraft = DocumentDraft | KeyDraft;
export interface Tab {
  id: string;
  name: string;
  connection: string;
  sql: string;
  kind?: "sql" | "key_value" | "document";
  draft?: NativeDraft;
}
export interface Preferences {
  theme: "dark" | "light";
  rowLimit: number;
  timeout: number;
  fontSize: number;
  sidebar: boolean;
  restoreNativeDrafts: boolean;
  shortcuts: Shortcuts;
  confirmations: Confirmations;
}
export const defaults: Preferences = {
  theme: "dark",
  rowLimit: 10_000,
  timeout: 60,
  fontSize: 13,
  sidebar: true,
  restoreNativeDrafts: false,
  shortcuts: { ...defaultShortcuts },
  confirmations: { ...defaultConfirmations },
};
function restoreDraft(
  value: unknown,
  kind: Tab["kind"],
): NativeDraft | undefined {
  if (typeof value !== "object" || value === null) return;
  const d = value as Record<string, unknown>;
  const strings = (keys: string[]) =>
    keys.every(
      (key) =>
        typeof d[key] === "string" &&
        new TextEncoder().encode(d[key]).byteLength <= 1024 * 1024,
    );
  if (
    kind === "document" &&
    d.kind === kind &&
    strings(["database", "collection", "search", "text", "sort"]) &&
    typeof d.aggregate === "boolean" &&
    typeof d.tree === "boolean"
  )
    return {
      kind,
      database: d.database as string,
      collection: d.collection as string,
      search: d.search as string,
      text: d.text as string,
      sort: d.sort as string,
      aggregate: d.aggregate,
      tree: d.tree,
    };
  if (
    kind === "key_value" &&
    d.kind === kind &&
    strings(["pattern", "command"])
  )
    return { kind, pattern: d.pattern as string, command: d.command as string };
}

export function withNativeDraft(tab: Tab, draft: NativeDraft): Tab {
  if (tab.kind !== draft.kind) return tab;
  if (
    tab.draft &&
    Object.entries(draft).every(
      ([key, value]) =>
        (tab.draft as unknown as Record<string, unknown>)[key] === value,
    )
  )
    return tab;
  return { ...tab, draft };
}

export function serializeWorkspace(
  tabs: Tab[],
  active: string,
  preferences: Preferences,
) {
  if (
    preferences.restoreNativeDrafts &&
    tabs.some((tab) => tab.draft && !restoreDraft(tab.draft, tab.kind))
  )
    throw new Error(
      "A MongoDB or Redis draft exceeds 1 MiB per text field or has an invalid shape. Shorten it or turn off draft restoration before saving.",
    );
  return {
    version: 2,
    tabs: tabs.map(({ id, name, connection, sql, kind, draft }) => ({
      id,
      name,
      connection,
      sql,
      kind,
      ...(preferences.restoreNativeDrafts
        ? { draft: restoreDraft(draft, kind) }
        : {}),
    })),
    active,
    preferences,
  };
}
export function restoreWorkspace(value: unknown): {
  tabs: Tab[];
  active: string;
  preferences: Preferences;
} {
  const data = value as {
    tabs?: unknown[];
    active?: string;
    preferences?: Partial<Preferences>;
  } | null;
  const tabs = Array.isArray(data?.tabs)
    ? data.tabs
        .filter(
          (t): t is Tab =>
            typeof t === "object" &&
            t !== null &&
            ["id", "name", "connection", "sql"].every(
              (k) => typeof (t as Record<string, unknown>)[k] === "string",
            ) &&
            (!("kind" in t) ||
              t.kind === "sql" ||
              t.kind === "key_value" ||
              t.kind === "document"),
        )
        .slice(0, 100)
        .map(({ id, name, connection, sql, kind, draft }) => ({
          id,
          name,
          connection,
          sql,
          kind,
          ...(data?.preferences?.restoreNativeDrafts === true
            ? { draft: restoreDraft(draft, kind) }
            : {}),
        }))
    : [];
  const p = data?.preferences;
  return {
    tabs,
    active: tabs.some((t) => t.id === data?.active)
      ? data!.active!
      : (tabs[0]?.id ?? ""),
    preferences: {
      ...defaults,
      theme: p?.theme === "light" ? "light" : "dark",
      rowLimit:
        typeof p?.rowLimit === "number" &&
        p.rowLimit >= 1 &&
        p.rowLimit <= 10_000_000
          ? p.rowLimit
          : defaults.rowLimit,
      timeout:
        typeof p?.timeout === "number" && p.timeout >= 1 && p.timeout <= 3600
          ? p.timeout
          : defaults.timeout,
      fontSize:
        typeof p?.fontSize === "number" && p.fontSize >= 10 && p.fontSize <= 24
          ? p.fontSize
          : defaults.fontSize,
      sidebar: p?.sidebar !== false,
      restoreNativeDrafts: p?.restoreNativeDrafts === true,
      shortcuts: restoreShortcuts(p?.shortcuts),
      confirmations: restoreConfirmations(p?.confirmations),
    },
  };
}

export function workspaceKind(
  capabilities: Capabilities | undefined,
  engine?: string,
): NonNullable<Tab["kind"]> {
  if (capabilities)
    return capabilities.document_queries
      ? "document"
      : capabilities.key_value
        ? "key_value"
        : "sql";
  return engine === "mongodb"
    ? "document"
    : engine === "redis"
      ? "key_value"
      : "sql";
}
