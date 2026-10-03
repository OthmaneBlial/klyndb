import { expect, test, vi } from "vitest";
import {
  defaultShortcuts,
  dispatchShortcut,
  restoreShortcuts,
  shortcutFromEvent,
  shortcutLabel,
} from "./shortcuts";
import { defaults, restoreWorkspace, serializeWorkspace } from "./workspace";

function event(key: string, patch: Partial<KeyboardEvent> = {}) {
  return {
    key,
    ctrlKey: true,
    metaKey: false,
    shiftKey: false,
    altKey: false,
    repeat: false,
    isComposing: false,
    defaultPrevented: false,
    getModifierState: () => false,
    preventDefault: vi.fn(),
    stopPropagation: vi.fn(),
    ...patch,
  };
}
test("dispatches exact remapped actions once and guards dialogs, composition, repeats and unavailable SQL", () => {
  const bindings = {
    ...defaultShortcuts,
    run: "Mod-Shift-Enter",
    runAll: "Mod-Alt-Enter",
  };
  const actions = { run: vi.fn(), runAll: vi.fn(), palette: vi.fn() };
  for (const modifiers of [
    { ctrlKey: true, metaKey: false },
    { ctrlKey: false, metaKey: true },
  ]) {
    const key = event("Enter", { ...modifiers, shiftKey: true });
    expect(dispatchShortcut(key, bindings, actions, false)).toBe(true);
    expect(key.preventDefault).toHaveBeenCalledOnce();
    expect(key.stopPropagation).toHaveBeenCalledOnce();
    expect(
      dispatchShortcut(event("Enter", modifiers), bindings, actions, false),
    ).toBe(false);
  }
  expect(actions.run).toHaveBeenCalledTimes(2);
  expect(actions.runAll).not.toHaveBeenCalled();
  expect(
    dispatchShortcut(
      event("Enter", { altKey: true }),
      bindings,
      actions,
      false,
    ),
  ).toBe(true);
  expect(actions.runAll).toHaveBeenCalledOnce();
  for (const patch of [
    { repeat: true },
    { isComposing: true },
    { defaultPrevented: true },
    { metaKey: true },
    { getModifierState: () => true },
  ]) {
    const key = event("Enter", { shiftKey: true, ...patch });
    expect(dispatchShortcut(key, bindings, actions, false)).toBe(false);
    expect(key.preventDefault).not.toHaveBeenCalled();
  }
  expect(
    dispatchShortcut(
      event("Enter", { shiftKey: true }),
      bindings,
      actions,
      true,
    ),
  ).toBe(false);
  expect(
    dispatchShortcut(event("Enter", { shiftKey: true }), bindings, {}, false),
  ).toBe(false);
  expect(actions.run).toHaveBeenCalledTimes(2);
  expect(shortcutFromEvent(event("a", { ctrlKey: false }))).toBeNull();
  expect(shortcutFromEvent(event("F6", { ctrlKey: false }))).toBe("F6");
  expect(shortcutFromEvent(event("K"))).toBe("Mod-k");
  expect(shortcutFromEvent(event("K", { shiftKey: true }))).toBe("Mod-Shift-k");
  expect(shortcutLabel("Mod-Shift-Enter", true)).toBe("⌘ ⇧ ↵");
  expect(shortcutLabel("Mod-Shift-Enter", false)).toBe("Ctrl ⇧ ↵");
});
test("persists disabled/custom bindings, restores old workspaces and rejects invalid/conflicting saved maps", () => {
  const shortcuts = {
    ...defaultShortcuts,
    run: "Mod-Shift-Enter",
    runAll: "F6",
    palette: "",
  };
  const saved = serializeWorkspace(
    [{ id: "sql", name: "SQL", connection: "c", sql: "SELECT 1; SELECT 2;" }],
    "sql",
    { ...defaults, shortcuts },
  );
  const restored = restoreWorkspace(JSON.parse(JSON.stringify(saved)));
  expect(restored.preferences.shortcuts).toEqual(shortcuts);
  expect(restored.tabs[0].sql).toBe("SELECT 1; SELECT 2;");
  expect(
    restoreWorkspace({ preferences: { theme: "light" } }).preferences.shortcuts,
  ).toEqual(defaultShortcuts);
  for (const value of [
    { run: 42 },
    { run: "Mod-q" },
    { run: "Mod-z" },
    { run: "Mod-k" },
    { run: "Enter" },
    { run: "Mod-Shift-Shift-k" },
  ])
    expect(restoreShortcuts(value)).toEqual(defaultShortcuts);
  const defaultsCopy = restoreShortcuts(null);
  defaultsCopy.run = "";
  expect(defaultShortcuts.run).toBe("Mod-Enter");
});
