import { useEffect, useRef, useState } from "react";
import { FolderOpen, FileCode2, Square } from "lucide-react";
import {
  api,
  type Connection,
  type ImportStatus,
  type SqlSource,
} from "../api";
import { Modal } from "./Modal";

export function SqlImportDialog({
  connection,
  timeout,
  onClose,
  onComplete,
}: {
  connection: Connection;
  timeout: number;
  onClose: () => void;
  onComplete: (status: ImportStatus) => void;
}) {
  const [source, setSource] = useState<SqlSource | null>(null);
  const [busy, setBusy] = useState(false);
  const [job, setJob] = useState<string | null>(null);
  const [status, setStatus] = useState<ImportStatus | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState("");
  const [cancelled, setCancelled] = useState(false);
  const sourceRef = useRef(source);
  sourceRef.current = source;
  const completeRef = useRef(onComplete);
  completeRef.current = onComplete;
  useEffect(
    () => () => {
      const id = sourceRef.current?.id;
      if (id) void api("release_import", { id }).catch(() => {});
    },
    [],
  );
  useEffect(() => {
    if (!job) return;
    let live = true;
    let pending = false;
    const poll = async () => {
      if (pending) return;
      pending = true;
      try {
        const next = await api("import_status", { id: job });
        if (!live) return;
        setStatus(next);
        if (next.done) {
          setError("");
          live = false;
          setJob(null);
          setBusy(false);
          completeRef.current(next);
        }
      } catch (e) {
        if (live)
          setError(
            `Could not read SQL import status: ${e}. Keep this dialog open and verify the connection.`,
          );
      } finally {
        pending = false;
      }
    };
    const timer = setInterval(() => void poll(), 200);
    void poll();
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [job]);
  async function close() {
    if (busy || job) {
      setError(
        "Wait for the operation to finish, or cancel and wait for execution to stop.",
      );
      return;
    }
    try {
      if (source) await api("release_import", { id: source.id });
      onClose();
    } catch (e) {
      setError(String(e));
    }
  }
  async function choose() {
    setBusy(true);
    setError("");
    setConfirmed(false);
    setStatus(null);
    setCancelled(false);
    try {
      if (source) await api("release_import", { id: source.id });
      setSource(null);
      setSource(
        await api("choose_sql_import_file", { connection: connection.id }),
      );
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function start() {
    if (!source || !confirmed) return;
    setBusy(true);
    setError("");
    try {
      setJob(
        await api("start_sql_import", {
          request: {
            source: source.id,
            connection: connection.id,
            timeout_seconds: timeout,
            confirmed,
          },
        }),
      );
    } catch (e) {
      setBusy(false);
      setError(String(e));
    }
  }
  return (
    <Modal
      title="Import SQL file"
      wide
      className="import-dialog"
      onClose={() => void close()}
    >
      <p>
        <strong>{connection.name}</strong> · {connection.engine} ·{" "}
        {connection.environment}
      </p>
      <p className="muted">
        Run the script on this connection without loading it into the editor.
        All SQL is checked before execution; SELECT results are
        discarded.
      </p>
      <div className="import-file">
        <button disabled={busy || !!job} onClick={() => void choose()}>
          <FolderOpen size={15} />{" "}
          {source ? "Choose another file" : "Choose SQL file"}
        </button>
        {source && (
          <span>
            <strong>{source.name}</strong>
            <small>
              {source.preview.statements.toLocaleString()} {source.preview.unit} ·{" "}
              {(source.bytes / 1024).toLocaleString(undefined, {
                maximumFractionDigits: 1,
              })}{" "}
              KiB
            </small>
          </span>
        )}
      </div>
      {!source && (
        <p className="muted">
          UTF-8 .sql · up to 512 MiB · 4 MiB per statement or SQL Server batch
        </p>
      )}
      {busy && !job && !source && (
        <p role="status">Copying and checking the SQL file…</p>
      )}
      {source && (
        <>
          <p className="import-section-label">
            First {source.preview.sample.length} {source.preview.unit}
          </p>
          <pre className="sql-preview">
            {source.preview.sample.join("\n\n")}
          </pre>
          {source.preview.warnings.map((w) => (
            <p className="error-text" key={w}>
              {w}
            </p>
          ))}
          <p>
            Statements and native batches run in order using the script’s own transaction commands.
            Earlier changes may stay committed after failure or cancellation.
            MySQL DDL can commit implicitly. An open or failed transaction needs
            COMMIT or ROLLBACK in the editor.
          </p>
          {!status && (
            <label className="check">
              <input
                type="checkbox"
                checked={confirmed}
                disabled={busy}
                onChange={(e) => setConfirmed(e.target.checked)}
              />{" "}
              I reviewed the file and allow it to change {connection.name}
              {connection.environment === "production" ? " (production)" : ""}.
            </label>
          )}
        </>
      )}
      {status && (
        <div role="status" aria-live="polite">
          <p>
            {status.completed_statements.toLocaleString()} of{" "}
            {source?.preview.statements.toLocaleString()} {source?.preview.unit} completed ·{" "}
            {(status.elapsed_ms / 1000).toFixed(1)} s
          </p>
          {status.done && !status.error && (
            <p>
              Script completed. Transaction:{" "}
              {status.transaction ?? "unavailable"}.
            </p>
          )}
          {status.error && <p className="error-text">{status.error}</p>}
          {status.done &&
            status.transaction &&
            status.transaction !== "idle" && (
              <p>
                Transaction {status.transaction}: review in the editor and{" "}
                {status.transaction === "failed"
                  ? "ROLLBACK"
                  : "COMMIT or ROLLBACK"}
                .
              </p>
            )}
        </div>
      )}
      {error && (
        <p className="error-text" role="alert">
          {error}
        </p>
      )}
      <footer>
        <button disabled={busy || !!job} onClick={() => void close()}>
          Close
        </button>
        {job ? (
          <button
            className="danger"
            disabled={cancelled}
            onClick={() =>
              void api("cancel_import", { id: job })
                .then(() => setCancelled(true))
                .catch((e) => setError(String(e)))
            }
          >
            <Square size={14} /> {cancelled ? "Stopping…" : "Cancel execution"}
          </button>
        ) : (
          <button
            className="primary"
            disabled={!source || !confirmed || busy || !!status}
            onClick={() => void start()}
          >
            <FileCode2 size={15} /> Execute file
          </button>
        )}
      </footer>
    </Modal>
  );
}
