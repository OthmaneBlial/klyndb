import { useState } from "react";
import { FolderOpen, Database, ShieldCheck, Plus } from "lucide-react";
import { api, type Connection } from "../api";
import { Modal } from "./Modal";
const fresh = (): Connection => ({
  id: "",
  name: "",
  engine: "sqlite",
  address: "",
  environment: "development",
  group: "",
  color: "#79c7a4",
  favorite: false,
  read_only: false,
  create_file: false,
});
export function ConnectionDialog({
  initial,
  onClose,
  onSaved,
}: {
  initial?: Connection;
  onClose: () => void;
  onSaved: (c: Connection, password: string | null, connect: boolean) => void;
}) {
  const [form, setForm] = useState(initial ?? fresh),
    [password, setPassword] = useState(""),
    [remember, setRemember] = useState(true),
    [busy, setBusy] = useState(false),
    [error, setError] = useState("");
  function field<K extends keyof Connection>(key: K, value: Connection[K]) {
    setForm((f) => ({ ...f, [key]: value }));
  }
  async function choose(create: boolean) {
    try {
      const path = await api("choose_database_file", { create });
      if (path) {
        field("address", path);
        field("create_file", create);
        if (!form.name)
          field("name", path.split(/[\\/]/).pop() ?? "Local database");
      }
    } catch (e) {
      setError(String(e));
    }
  }
  async function save(connect: boolean) {
    setBusy(true);
    setError("");
    try {
      let secret = password;
      try {
        secret ||= decodeURIComponent(new URL(form.address).password);
      } catch {
        /* Local file paths are not URLs. */
      }
      const connection = await api("save_connection", {
        connection: form,
        password: secret || null,
        remember,
      });
      onSaved(connection, secret || null, connect);
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <Modal
      title={initial ? "Edit connection" : "New connection"}
      onClose={onClose}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void save(true);
        }}
      >
        <div className="engine-picker">
          {["sqlite", "postgres"].map((engine) => (
            <button
              type="button"
              key={engine}
              className={form.engine === engine ? "selected" : ""}
              onClick={() =>
                setForm((f) => ({
                  ...f,
                  engine,
                  address: "",
                  create_file: false,
                }))
              }
            >
              <Database size={20} />
              <strong>{engine === "sqlite" ? "SQLite" : "PostgreSQL"}</strong>
              <span>
                {engine === "sqlite" ? "Local file" : "Server connection"}
              </span>
            </button>
          ))}
        </div>
        <label>
          Name
          <input
            value={form.name}
            onChange={(e) => field("name", e.target.value)}
            autoFocus
            placeholder="e.g. Analytics · local"
            required
            maxLength={200}
          />
        </label>
        {form.engine === "sqlite" ? (
          <label>
            Database file
            <div className="input-action">
              <input
                value={form.address}
                onChange={(e) => field("address", e.target.value)}
                placeholder="/path/to/database.sqlite"
                required
              />
              <button
                type="button"
                aria-label="Browse database file"
                onClick={() => void choose(false)}
              >
                <FolderOpen size={17} />
              </button>
              <button
                type="button"
                title="Create database file"
                onClick={() => void choose(true)}
              >
                <Plus size={17} />
              </button>
            </div>
            <small>
              {form.create_file
                ? "A new database will be created when you connect."
                : "Choose an existing SQLite database, or use + to create one."}
            </small>
          </label>
        ) : (
          <>
            <label>
              Connection URL
              <input
                value={form.address}
                onChange={(e) => field("address", e.target.value)}
                placeholder="postgresql://user@localhost:5432/database"
                required
                autoComplete="off"
              />
              <small>
                TLS verification is enabled by default. Add sslmode=disable only
                for a trusted local server.
              </small>
            </label>
            <label>
              Password
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                placeholder={
                  initial
                    ? "Leave empty to keep stored password"
                    : "Database password"
                }
                autoComplete="new-password"
              />
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={remember}
                onChange={(e) => setRemember(e.target.checked)}
              />
              <ShieldCheck size={15} /> Store password in the OS keychain
            </label>
          </>
        )}
        <div className="form-row">
          <label>
            Environment
            <select
              value={form.environment}
              onChange={(e) => field("environment", e.target.value)}
            >
              <option value="development">Development</option>
              <option value="staging">Staging</option>
              <option value="production">Production</option>
            </select>
          </label>
          <label>
            Group
            <input
              value={form.group}
              onChange={(e) => field("group", e.target.value)}
              placeholder="Personal"
            />
          </label>
          <label className="color-label">
            Label
            <input
              type="color"
              value={form.color}
              onChange={(e) => field("color", e.target.value)}
            />
          </label>
        </div>
        <label className="check">
          <input
            type="checkbox"
            checked={form.read_only}
            onChange={(e) => field("read_only", e.target.checked)}
          />{" "}
          Read-only connection
        </label>
        <label className="check">
          <input
            type="checkbox"
            checked={form.favorite}
            onChange={(e) => field("favorite", e.target.checked)}
          />{" "}
          Favorite
        </label>
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
        <footer>
          <button
            type="button"
            disabled={busy}
            onClick={() => void save(false)}
          >
            Save
          </button>
          <button className="primary" type="submit" disabled={busy}>
            {busy ? "Saving…" : "Save & connect"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
