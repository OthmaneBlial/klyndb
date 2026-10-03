import {
  defaultConfirmations,
  destructiveConfirmations,
  type Confirmations,
} from "../confirmations";
import { useState, type Dispatch, type SetStateAction } from "react";
import { Moon, Sun, Shield } from "lucide-react";
import type { Preferences } from "../workspace";
import { Modal } from "./Modal";
import {
  defaultShortcuts,
  shortcutDefinitions,
  shortcutFromEvent,
  shortcutLabel,
  validShortcut,
} from "../shortcuts";
export function SettingsDialog({
  preferences,
  setPreferences,
  onClose,
}: {
  preferences: Preferences;
  setPreferences: Dispatch<SetStateAction<Preferences>>;
  onClose: () => void;
}) {
  const [shortcutNotice, setShortcutNotice] = useState("");
  return (
    <Modal title="Workspace settings" onClose={onClose}>
      <div className="settings-form">
        <label>
          Appearance
          <div className="segmented">
            <button
              className={preferences.theme === "dark" ? "selected" : ""}
              onClick={() => setPreferences((p) => ({ ...p, theme: "dark" }))}
            >
              <Moon size={15} /> Dark
            </button>
            <button
              className={preferences.theme === "light" ? "selected" : ""}
              onClick={() => setPreferences((p) => ({ ...p, theme: "light" }))}
            >
              <Sun size={15} /> Light
            </button>
          </div>
        </label>
        <label>
          Result row limit
          <input
            type="number"
            min={1}
            max={10_000_000}
            value={preferences.rowLimit}
            onChange={(e) =>
              setPreferences((p) => ({
                ...p,
                rowLimit: Math.max(
                  1,
                  Math.min(10_000_000, Number(e.target.value)),
                ),
              }))
            }
          />
          <small>
            Per statement. Results stay on disk; only a visible page enters the
            UI.
          </small>
        </label>
        <label>
          Query timeout (seconds)
          <input
            type="number"
            min={1}
            max={3600}
            value={preferences.timeout}
            onChange={(e) =>
              setPreferences((p) => ({
                ...p,
                timeout: Math.max(1, Math.min(3600, Number(e.target.value))),
              }))
            }
          />
        </label>
        <label>
          Editor font size
          <input
            type="number"
            min={10}
            max={24}
            value={preferences.fontSize}
            onChange={(e) =>
              setPreferences((p) => ({
                ...p,
                fontSize: Math.max(10, Math.min(24, Number(e.target.value))),
              }))
            }
          />
        </label>
        <label>
          <span className="check">
            <input
              type="checkbox"
              checked={preferences.restoreNativeDrafts}
              onChange={(e) =>
                setPreferences((p) => ({
                  ...p,
                  restoreNativeDrafts: e.target.checked,
                }))
              }
            />
            Restore MongoDB and Redis drafts after restart
          </span>
          <small>
            Off by default. Saves query text, target collections, key patterns
            and command arguments locally without encryption; literals may be
            sensitive. Results and document-edit drafts are excluded. Turning
            this off removes saved drafts on the next workspace save and keeps
            current drafts in memory. Restored requests never run automatically.
          </small>
        </label>
        <details className="shortcut-settings confirmation-settings">
          <summary>Query confirmations</summary>
          <label>
            Production SQL
            <select
              value={preferences.confirmations.production}
              onChange={(e) =>
                setPreferences((p) => ({
                  ...p,
                  confirmations: {
                    ...p.confirmations,
                    production: e.target.value as Confirmations["production"],
                  },
                }))
              }
            >
              <option value="writes">Confirm writes</option>
              <option value="all">Confirm every query</option>
              <option value="destructive">
                Use destructive SQL rules only
              </option>
            </select>
          </label>
          <p className="muted">
            Ask before executing these statements on any connection:
          </p>
          {destructiveConfirmations.map(({ id, label }) => (
            <label className="check" key={id}>
              <input
                type="checkbox"
                checked={preferences.confirmations[id]}
                onChange={(e) =>
                  setPreferences((p) => ({
                    ...p,
                    confirmations: {
                      ...p.confirmations,
                      [id]: e.target.checked,
                    },
                  }))
                }
              />
              {label}
            </label>
          ))}
          <p className="muted">
            Turning a rule off lets matching SQL run without that prompt.
            Read-only connections still reject writes. ANALYZE, reviewed edits,
            imports and reconnect keep their own confirmations.
          </p>
          <button
            onClick={() =>
              setPreferences((p) => ({
                ...p,
                confirmations: { ...defaultConfirmations },
              }))
            }
          >
            Restore confirmation defaults
          </button>
        </details>
        <details className="shortcut-settings">
          <summary>Keyboard shortcuts</summary>
          <p className="muted">
            Focus a binding and press Cmd/Ctrl with a letter, number or Enter,
            or use F1–F12. Shift and Alt are optional. Clear disables a
            shortcut; buttons and commands still work. Changes save locally.
          </p>
          {shortcutDefinitions.map(({ id, name }) => (
            <div className="shortcut-row" key={id}>
              <label htmlFor={`shortcut-${id}`}>{name}</label>
              <input
                id={`shortcut-${id}`}
                readOnly
                value={shortcutLabel(preferences.shortcuts[id])}
                placeholder="Unassigned"
                onFocus={() => setShortcutNotice("")}
                onKeyDown={(event) => {
                  if (
                    [
                      "Tab",
                      "Escape",
                      "Shift",
                      "Control",
                      "Alt",
                      "Meta",
                    ].includes(event.key)
                  )
                    return;
                  event.preventDefault();
                  event.stopPropagation();
                  const key = shortcutFromEvent(event.nativeEvent);
                  if (!key || !validShortcut(key)) {
                    setShortcutNotice(
                      "Choose a supported combination. Standard editing and window shortcuts are reserved.",
                    );
                    return;
                  }
                  const conflict = shortcutDefinitions.find(
                    (command) =>
                      command.id !== id &&
                      preferences.shortcuts[command.id] === key,
                  );
                  if (conflict) {
                    setShortcutNotice(
                      `Already assigned to ${conflict.name}. Clear that binding first or choose another combination.`,
                    );
                    return;
                  }
                  setPreferences((p) => ({
                    ...p,
                    shortcuts: { ...p.shortcuts, [id]: key },
                  }));
                  setShortcutNotice(`${name}: ${shortcutLabel(key)}`);
                }}
              />
              <button
                aria-label={`Clear shortcut for ${name}`}
                disabled={!preferences.shortcuts[id]}
                onClick={() => {
                  setPreferences((p) => ({
                    ...p,
                    shortcuts: { ...p.shortcuts, [id]: "" },
                  }));
                  setShortcutNotice(`${name}: shortcut disabled.`);
                }}
              >
                Clear
              </button>
            </div>
          ))}
          <p role="status">{shortcutNotice}</p>
          <button
            onClick={() => {
              setPreferences((p) => ({
                ...p,
                shortcuts: { ...defaultShortcuts },
              }));
              setShortcutNotice("Default shortcuts restored.");
            }}
          >
            Restore shortcut defaults
          </button>
        </details>
        <div className="privacy-note">
          <Shield size={18} />
          <p>
            No account, telemetry, schema upload or cloud dependency. SQL
            history is stored locally and can be cleared from Query history.
          </p>
        </div>
      </div>
    </Modal>
  );
}
