export const shortcutDefinitions = [
  { id: "palette", name: "Command palette", key: "Mod-k" },
  { id: "newTab", name: "New tab", key: "Mod-t" },
  { id: "newConnection", name: "New connection", key: "" },
  { id: "run", name: "Run current statement or selection", key: "Mod-Enter" },
  { id: "runAll", name: "Run entire editor", key: "" },
  { id: "format", name: "Format SQL", key: "Mod-Shift-f" },
  { id: "save", name: "Save query", key: "Mod-s" },
  { id: "refresh", name: "Refresh schema", key: "" },
  { id: "saved", name: "Saved queries", key: "" },
  { id: "history", name: "Query history", key: "" },
  { id: "sidebar", name: "Toggle sidebar", key: "" },
  { id: "settings", name: "Settings", key: "" },
] as const;
export type ShortcutAction = (typeof shortcutDefinitions)[number]["id"];
export type Shortcuts = Record<ShortcutAction, string>;
export const defaultShortcuts = Object.fromEntries(
  shortcutDefinitions.map(({ id, key }) => [id, key]),
) as Shortcuts;

const reserved = new Set([
  "Mod-a",
  "Mod-c",
  "Mod-x",
  "Mod-v",
  "Mod-z",
  "Mod-Shift-z",
  "Mod-y",
  "Mod-f",
  "Mod-q",
  "Mod-w",
  "Mod-h",
  "Mod-m",
  "Mod-r",
  "Mod-l",
]);
export function validShortcut(key: string) {
  return (
    key === "" ||
    (!reserved.has(key) &&
      /^(Mod-(Shift-)?(Alt-)?([a-z0-9]|Enter|F([1-9]|1[0-2]))|(Shift-)?(Alt-)?F([1-9]|1[0-2]))$/.test(
        key,
      ))
  );
}
export function restoreShortcuts(value: unknown): Shortcuts {
  if (!value || typeof value !== "object") return { ...defaultShortcuts };
  const data = value as Record<string, unknown>;
  const restored = { ...defaultShortcuts };
  for (const { id } of shortcutDefinitions) {
    if (data[id] === undefined) continue;
    if (typeof data[id] !== "string" || !validShortcut(data[id]))
      return { ...defaultShortcuts };
    restored[id] = data[id];
  }
  const enabled = Object.values(restored).filter(Boolean);
  return new Set(enabled).size === enabled.length
    ? restored
    : { ...defaultShortcuts };
}
type KeyEvent = Pick<
  KeyboardEvent,
  | "key"
  | "metaKey"
  | "ctrlKey"
  | "shiftKey"
  | "altKey"
  | "isComposing"
  | "repeat"
  | "defaultPrevented"
  | "getModifierState"
  | "preventDefault"
  | "stopPropagation"
>;
export function shortcutFromEvent(event: KeyEvent): string | null {
  if (
    event.isComposing ||
    (event.ctrlKey && event.metaKey) ||
    event.getModifierState("AltGraph")
  )
    return null;
  const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;
  const mod = event.metaKey || event.ctrlKey;
  if (
    !/^F([1-9]|1[0-2])$/.test(key) &&
    (!mod || !/^([a-z0-9]|Enter)$/.test(key))
  )
    return null;
  return `${mod ? "Mod-" : ""}${event.shiftKey ? "Shift-" : ""}${event.altKey ? "Alt-" : ""}${key}`;
}
export function shortcutLabel(
  key: string,
  mac = typeof navigator !== "undefined" && /Mac/.test(navigator.platform),
) {
  return key
    .replace("Mod-", mac ? "⌘ " : "Ctrl ")
    .replace("Shift-", "⇧ ")
    .replace("Alt-", mac ? "⌥ " : "Alt ")
    .replace("Enter", "↵")
    .replace(/[a-z]$/, (letter) => letter.toUpperCase());
}
export function dispatchShortcut(
  event: KeyEvent,
  bindings: Shortcuts,
  actions: Partial<Record<ShortcutAction, () => void | Promise<void>>>,
  modalOpen: boolean,
) {
  if (modalOpen || event.repeat || event.isComposing || event.defaultPrevented)
    return false;
  const key = shortcutFromEvent(event);
  if (!key) return false;
  const command = shortcutDefinitions.find(({ id }) => bindings[id] === key);
  const action = command && actions[command.id];
  if (!action) return false;
  event.preventDefault();
  event.stopPropagation();
  void action();
  return true;
}
