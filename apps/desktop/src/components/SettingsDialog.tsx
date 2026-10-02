import type { Dispatch, SetStateAction } from "react";
import { Moon, Sun, Shield } from "lucide-react";
import type { Preferences } from "../workspace";
import { Modal } from "./Modal";
export function SettingsDialog({
  preferences,
  setPreferences,
  onClose,
}: {
  preferences: Preferences;
  setPreferences: Dispatch<SetStateAction<Preferences>>;
  onClose: () => void;
}) {
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
