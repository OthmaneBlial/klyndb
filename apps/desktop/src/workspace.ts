export interface Tab {
  id: string;
  name: string;
  connection: string;
  sql: string;
  kind?: "sql" | "key_value";
}
export interface Preferences {
  theme: "dark" | "light";
  rowLimit: number;
  timeout: number;
  fontSize: number;
  sidebar: boolean;
}
export const defaults: Preferences = {
  theme: "dark",
  rowLimit: 10_000,
  timeout: 60,
  fontSize: 13,
  sidebar: true,
};
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
            (!("kind" in t) || t.kind === "sql" || t.kind === "key_value"),
        )
        .slice(0, 100)
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
    },
  };
}
