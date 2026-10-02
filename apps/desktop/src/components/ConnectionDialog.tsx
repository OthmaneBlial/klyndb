import { useState } from "react";
import { FolderOpen, Database, ShieldCheck, Plus } from "lucide-react";
import { api, type Connection } from "../api";
import { Modal } from "./Modal";
import {
  connectionTimeout,
  updateConnectTimeout,
  tlsSettings,
  updateTls,
} from "../connection";
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
  onSaved: (
    c: Connection,
    password: string | null,
    connect: boolean,
    identityPassword: string | null,
  ) => void;
}) {
  const [form, setForm] = useState(initial ?? fresh),
    [password, setPassword] = useState(""),
    [remember, setRemember] = useState(true),
    [identityPassword, setIdentityPassword] = useState(""),
    [rememberIdentity, setRememberIdentity] = useState(true),
    [busy, setBusy] = useState(false),
    [error, setError] = useState(""),
    [testStatus, setTestStatus] = useState("");
  function field<K extends keyof Connection>(key: K, value: Connection[K]) {
    setTestStatus("");
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
  const tls = tlsSettings(form.engine, form.address);
  function changeTls(mode: string, ca: string, identity?: string) {
    try {
      field(
        "address",
        updateTls(form.engine, form.address, mode, ca, identity),
      );
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }
  async function chooseCa() {
    try {
      const path = await api("choose_ca_file");
      if (path)
        changeTls(form.engine === "mysql" ? "required" : "require", path);
    } catch (e) {
      setError(String(e));
    }
  }
  async function chooseIdentity() {
    try {
      const path = await api("choose_client_identity_file");
      if (path) {
        changeTls(
          form.engine === "mysql" ? "required" : "require",
          tls.ca,
          path,
        );
        setIdentityPassword("");
      }
    } catch (e) {
      setError(String(e));
    }
  }
  async function test() {
    setBusy(true);
    setError("");
    setTestStatus("");
    try {
      await api("test_connection", {
        connection: form,
        password: password || null,
        identityPassword: tls.identity ? identityPassword || null : null,
      });
      setTestStatus("Connection verified. The test session has been closed.");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function save(connect: boolean) {
    setBusy(true);
    setError("");
    setTestStatus("");
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
        identityPassword: tls.identity ? identityPassword || null : null,
        rememberIdentity,
      });
      onSaved(
        connection,
        secret || null,
        connect,
        tls.identity ? identityPassword || null : null,
      );
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
        <fieldset
          className="connection-fields"
          disabled={busy}
          aria-label="Connection details"
        >
          <div className="engine-picker">
            {["sqlite", "postgres", "mysql"].map((engine) => (
              <button
                type="button"
                key={engine}
                className={form.engine === engine ? "selected" : ""}
                onClick={() => {
                  setTestStatus("");
                  setIdentityPassword("");
                  setForm((f) => ({
                    ...f,
                    engine,
                    address: "",
                    create_file: false,
                  }));
                }}
              >
                <Database size={20} />
                <strong>
                  {engine === "sqlite"
                    ? "SQLite"
                    : engine === "mysql"
                      ? "MySQL / MariaDB"
                      : "PostgreSQL"}
                </strong>
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
                  placeholder={
                    form.engine === "mysql"
                      ? "mysql://user@localhost:3306/database"
                      : "postgresql://user@localhost:5432/database"
                  }
                  required
                  autoComplete="off"
                />
                <small>
                  TLS verification is enabled by default. Add{" "}
                  {form.engine === "mysql" ? "tls=disabled" : "sslmode=disable"}{" "}
                  only for a trusted local server.
                </small>
              </label>
              <details className="tls-options">
                <summary>Network</summary>
                <label>
                  Connection timeout · seconds
                  <input
                    type="number"
                    aria-label="Connection timeout in seconds"
                    min={1}
                    max={300}
                    step={1}
                    required
                    value={connectionTimeout(form.address)}
                    onChange={(e) => {
                      try {
                        field(
                          "address",
                          updateConnectTimeout(form.address, e.target.value),
                        );
                        setError("");
                      } catch (error) {
                        setError(String(error));
                      }
                    }}
                  />
                  <small>
                    1–300 seconds, default 10. Applies when connecting or
                    testing; query timeout is separate.
                  </small>
                </label>
              </details>
              <details className="tls-options">
                <summary>TLS &amp; certificates</summary>
                <label>
                  Transport
                  <select
                    aria-label="TLS transport"
                    value={tls.mode}
                    onChange={(e) => changeTls(e.target.value, tls.ca)}
                  >
                    <option
                      value={form.engine === "mysql" ? "required" : "require"}
                    >
                      Verified TLS (default)
                    </option>
                    {form.engine === "postgres" && (
                      <option
                        value="prefer"
                        disabled={!!(tls.ca || tls.identity)}
                      >
                        Try TLS, allow plaintext fallback
                      </option>
                    )}
                    <option
                      value={form.engine === "mysql" ? "disabled" : "disable"}
                      disabled={!!(tls.ca || tls.identity)}
                    >
                      Plaintext · trusted local server only
                    </option>
                  </select>
                </label>
                <label>
                  Custom CA certificates · optional
                  <div className="input-action">
                    <input
                      aria-label="CA certificate file"
                      value={tls.ca}
                      placeholder="System trust store"
                      onChange={(e) =>
                        changeTls(
                          form.engine === "mysql" ? "required" : "require",
                          e.target.value,
                        )
                      }
                    />
                    <button
                      type="button"
                      aria-label="Choose CA certificate file"
                      onClick={() => void chooseCa()}
                    >
                      <FolderOpen size={17} />
                    </button>
                    {tls.ca && (
                      <button
                        type="button"
                        onClick={() => changeTls(tls.mode, "")}
                      >
                        Clear
                      </button>
                    )}
                  </div>
                  <small>
                    PEM bundle or DER file, up to 1 MiB. Saved as a file path.
                    Certificate and hostname checks stay enabled; a custom CA
                    requires TLS.
                  </small>
                </label>
                <label>
                  Client identity · optional
                  <div className="input-action">
                    <input
                      aria-label="Client identity file"
                      value={tls.identity}
                      placeholder="PKCS#12 (.p12 / .pfx)"
                      onChange={(e) => {
                        changeTls(
                          form.engine === "mysql" ? "required" : "require",
                          tls.ca,
                          e.target.value,
                        );
                        setIdentityPassword("");
                      }}
                    />
                    <button
                      type="button"
                      aria-label="Choose client identity file"
                      onClick={() => void chooseIdentity()}
                    >
                      <FolderOpen size={17} />
                    </button>
                    {tls.identity && (
                      <button
                        type="button"
                        onClick={() => {
                          changeTls(tls.mode, tls.ca, "");
                          setIdentityPassword("");
                        }}
                      >
                        Clear
                      </button>
                    )}
                  </div>
                  <small>
                    Certificate chain and private key in one PKCS#12 file, up to
                    1 MiB. Rust reads the file; keys stay outside the interface.
                  </small>
                </label>
                {tls.identity && (
                  <>
                    <label>
                      Certificate password
                      <input
                        type="password"
                        aria-label="Client certificate password"
                        autoComplete="off"
                        maxLength={16384}
                        value={identityPassword}
                        placeholder={
                          initial
                            ? "Leave empty to use the stored certificate password"
                            : "Password for the PKCS#12 file · optional"
                        }
                        onChange={(e) => {
                          setIdentityPassword(e.target.value);
                          setTestStatus("");
                        }}
                      />
                    </label>
                    <label className="check">
                      <input
                        type="checkbox"
                        checked={rememberIdentity}
                        onChange={(e) => setRememberIdentity(e.target.checked)}
                      />{" "}
                      Store certificate password in the OS keychain
                    </label>
                  </>
                )}
              </details>
              <label>
                Password
                <input
                  type="password"
                  value={password}
                  onChange={(e) => {
                    setPassword(e.target.value);
                    setTestStatus("");
                  }}
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
        </fieldset>
        {testStatus && <p role="status">{testStatus}</p>}
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
        <footer>
          <button type="button" disabled={busy} onClick={() => void test()}>
            {busy ? "Working…" : "Test connection"}
          </button>
          <button
            type="button"
            disabled={busy}
            onClick={() => void save(false)}
          >
            Save
          </button>
          <button className="primary" type="submit" disabled={busy}>
            {busy ? "Working…" : "Save & connect"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
