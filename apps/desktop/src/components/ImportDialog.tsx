import { useEffect, useRef, useState } from "react";
import { ArrowRight, FileUp, FolderOpen, Shield, Square } from "lucide-react";
import {
  api,
  type Connection,
  type ImportFormat,
  type ImportOptions,
  type ImportMapping,
  type ImportSource,
  type ImportStatus,
  type ImportValueKind,
  type Table,
  type TableInfo,
} from "../api";
import { importValueKind, previewValue, suggestMapping } from "../import";
import { Modal } from "./Modal";

export function ImportDialog({
  connection,
  table,
  info,
  timeout,
  onComplete,
  onClose,
}: {
  connection: Connection;
  table: Table;
  info: TableInfo;
  timeout: number;
  onComplete: (status: ImportStatus) => void;
  onClose: () => void;
}) {
  const [options, setOptions] = useState<ImportOptions>({
    format: "csv",
    delimiter: ",",
    trim: false,
    null_value: null,
    empty_as_null: false,
  });
  const [source, setSource] = useState<ImportSource | null>(null);
  const sourceRef = useRef<ImportSource | null>(null);
  sourceRef.current = source;
  const [mapping, setMapping] = useState<ImportMapping[]>([]);
  const [selected, setSelected] = useState(0);
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);
  const [confirmed, setConfirmed] = useState(false);
  const [status, setStatus] = useState<ImportStatus | null>(null);
  const [job, setJob] = useState<string | null>(null);
  const [cancelled, setCancelled] = useState(false);
  const [error, setError] = useState("");
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
            `Could not read import status: ${e}. Keep this dialog open and verify the connection.`,
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
        "Wait for the current operation, or cancel the import and wait for rollback.",
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
  function changeOptions(next: ImportOptions) {
    setOptions(next);
    setReady(false);
    setConfirmed(false);
    setError("");
  }
  async function choose() {
    setBusy(true);
    setError("");
    setConfirmed(false);
    try {
      if (source) await api("release_import", { id: source.id });
      setSource(null);
      setStatus(null);
      setCancelled(false);
      setMapping([]);
      setReady(false);
      const file = await api("choose_import_file", { options });
      if (file) {
        setSource(file);
        setMapping(suggestMapping(file.preview.headers, info.columns));
        setSelected(0);
        setReady(true);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function preview() {
    if (!source) return;
    setBusy(true);
    setError("");
    try {
      const next = await api("preview_import", { id: source.id, options });
      if (
        JSON.stringify(next.headers) !== JSON.stringify(source.preview.headers)
      ) {
        setMapping(suggestMapping(next.headers, info.columns));
        setSelected(0);
      }
      setSource({ ...source, preview: next });
      setReady(true);
    } catch (e) {
      setReady(false);
      setError(String(e));
    } finally {
      setBusy(false);
    }
  }
  async function start() {
    if (!source || busy || job || status || !ready) return;
    setBusy(true);
    setError("");
    try {
      const id = await api("start_import", {
        request: {
          source: source.id,
          connection: connection.id,
          table,
          options,
          mapping,
          timeout_seconds: timeout,
          confirmed,
        },
      });
      setStatus({
        id,
        connection_id: connection.id,
        done: false,
        read_rows: 0,
        completed_statements: 0,
        elapsed_ms: 0,
        result: null,
        transaction: null,
        error: null,
      });
      setJob(id);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }
  async function cancel() {
    if (!job) return;
    try {
      await api("cancel_import", { id: job });
      setCancelled(true);
      setError("");
    } catch (e) {
      setError(String(e));
    }
  }
  const chosen = mapping[selected];
  const mapped = mapping.filter((m) => m.column !== null);
  const duplicates =
    new Set(mapped.map((m) => m.column)).size !== mapped.length;
  const destination = info.columns.find((c) => c.name === chosen?.column);
  const patch = (change: Partial<ImportMapping>) => {
    setMapping((values) =>
      values.map((m, i) => (i === selected ? { ...m, ...change } : m)),
    );
    setConfirmed(false);
  };
  const rawValue = (value: string | null) => {
    const text = previewValue(value, options);
    return text === null ? (
      <span className="null-cell">NULL</span>
    ) : text === "" ? (
      <em className="muted">empty text</em>
    ) : (
      text
    );
  };
  return (
    <Modal title="Import data" onClose={() => void close()} wide>
      <div className="import-target">
        <FileUp size={22} strokeWidth={1.5} />
        <div>
          <strong>
            {table.schema}.{table.name}
          </strong>
          <small>
            {connection.name} · {connection.environment} · append rows
          </small>
        </div>
        {connection.environment === "production" && (
          <span className="environment production">
            <Shield size={13} /> Production
          </span>
        )}
      </div>
      <fieldset className="import-settings" disabled={busy || !!status}>
        <label className="import-format">
          Format
          <select
            value={options.format}
            onChange={(e) =>
              changeOptions({
                ...options,
                format: e.target.value as ImportFormat,
              })
            }
          >
            <option value="csv">CSV</option>
            <option value="json">JSON · object array</option>
            <option value="klyndb_json">JSON · Klyndb export</option>
          </select>
        </label>
        {options.format === "csv" && (
          <>
            <label>
              Separator
              <select
                value={options.delimiter}
                onChange={(e) =>
                  changeOptions({ ...options, delimiter: e.target.value })
                }
              >
                <option value=",">Comma</option>
                <option value=";">Semicolon</option>
                <option value={"\t"}>Tab</option>
                <option value="|">Pipe</option>
              </select>
            </label>
            <label>
              NULL token
              <input
                maxLength={256}
                placeholder="None"
                value={options.null_value ?? ""}
                onChange={(e) =>
                  changeOptions({
                    ...options,
                    null_value: e.target.value || null,
                  })
                }
              />
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={options.trim}
                onChange={(e) =>
                  changeOptions({ ...options, trim: e.target.checked })
                }
              />{" "}
              Trim whitespace
            </label>
            <label className="check">
              <input
                type="checkbox"
                checked={options.empty_as_null}
                onChange={(e) =>
                  changeOptions({ ...options, empty_as_null: e.target.checked })
                }
              />{" "}
              Empty fields as NULL
            </label>
          </>
        )}
      </fieldset>
      {options.format !== "csv" && (
        <p className="muted import-help">
          {options.format === "json"
            ? "An array of objects with matching field names. JSON null stays NULL. Numbers stay exact in Rust; database types still apply."
            : "An array of Klyndb export records with columns and typed values. Review destination value types before importing."}
        </p>
      )}
      <div className="import-file">
        <button onClick={() => void choose()} disabled={busy || !!job}>
          <FolderOpen size={15} />{" "}
          {source ? "Choose another file" : "Choose file"}
        </button>
        {source ? (
          <span>
            <strong>{source.name}</strong>
            <small>
              {(source.bytes / 1024).toLocaleString(undefined, {
                maximumFractionDigits: 1,
              })}{" "}
              KiB · {source.preview.headers.length} fields
            </small>
          </span>
        ) : (
          <small className="muted">
            UTF-8 {options.format === "csv" ? "CSV with headers" : "JSON array"}{" "}
            · up to 512 MiB
          </small>
        )}
        {source && !ready && !status && (
          <button
            className="primary"
            disabled={busy}
            onClick={() => void preview()}
          >
            Update preview
          </button>
        )}
      </div>
      {source && mapping.length > 0 && (
        <>
          <div className="import-section-label">
            <span>Map fields</span>
            <small>
              {mapped.length} of {mapping.length} mapped
            </small>
          </div>
          <div className="import-mapping">
            <nav aria-label="Source fields">
              {source.preview.headers.map((header, i) => (
                <button
                  key={i}
                  className={selected === i ? "selected" : ""}
                  aria-pressed={selected === i}
                  onClick={() => setSelected(i)}
                >
                  <span>
                    {i + 1}. {header}
                  </span>
                  <small>{mapping[i]?.column ?? "Ignore"}</small>
                </button>
              ))}
            </nav>
            <fieldset disabled={busy || !!status}>
              <label>
                Destination for {source.preview.headers[selected]}
                <select
                  value={chosen?.column ?? ""}
                  onChange={(e) =>
                    patch({
                      column: e.target.value || null,
                      kind: importValueKind(
                        info.columns.find((c) => c.name === e.target.value)
                          ?.data_type ?? "TEXT",
                      ),
                    })
                  }
                >
                  <option value="">Ignore this field</option>
                  {info.columns
                    .filter((c) => !c.generated)
                    .map((c) => (
                      <option
                        key={c.name}
                        value={c.name}
                        disabled={mapping.some(
                          (m, i) => i !== selected && m.column === c.name,
                        )}
                      >
                        {c.name} · {c.data_type}
                      </option>
                    ))}
                </select>
              </label>
              <label>
                Value type for {source.preview.headers[selected]}
                <select
                  value={chosen?.kind ?? "text"}
                  disabled={!chosen?.column}
                  onChange={(e) =>
                    patch({ kind: e.target.value as ImportValueKind })
                  }
                >
                  <option value="text">Text / server value</option>
                  <option value="number">Number</option>
                  <option value="boolean">Boolean</option>
                  <option value="binary">Binary (hex)</option>
                  <option value="json">JSON</option>
                </select>
              </label>
              <small className="muted">
                {destination
                  ? `${destination.data_type} · ${destination.nullable ? "nullable" : "not null"}${destination.default ? ` · default: ${destination.default}` : ""}`
                  : "This source field will not be imported."}
              </small>
              <div className="import-samples">
                <small>FIRST FIVE RECORDS</small>
                {source.preview.rows.map((row, i) => (
                  <div key={i}>
                    <span>{i + 1}</span>
                    <code>{rawValue(row[selected] ?? null)}</code>
                  </div>
                ))}
              </div>
            </fieldset>
          </div>
          <p className="muted import-help">
            Unmapped destination columns use database defaults. Generated
            columns are excluded.{" "}
            {source.preview.clipped
              ? "Preview values are clipped; full values are validated during import."
              : "Full records are validated during import."}
          </p>
          <details className="import-preview">
            <summary>
              Preview records <ArrowRight size={13} />
            </summary>
            <div>
              <table>
                <thead>
                  <tr>
                    {source.preview.headers.map((h, i) => (
                      <th key={i}>
                        {h}
                        <small>{mapping[i]?.column ?? "Ignore"}</small>
                      </th>
                    ))}
                  </tr>
                </thead>
                <tbody>
                  {source.preview.rows.map((row, i) => (
                    <tr key={i}>
                      {row.map((value, j) => (
                        <td key={j}>{rawValue(value)}</td>
                      ))}
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </details>
        </>
      )}
      {!status && source && (
        <p className="import-policy">
          Append only · existing rows stay in place · {timeout}s deadline.
          Errors trigger rollback. Successful imports in a manual transaction
          remain uncommitted.
        </p>
      )}
      {!status && source && connection.environment === "production" && (
        <label className="check import-confirm">
          <input
            type="checkbox"
            checked={confirmed}
            disabled={busy || !ready}
            onChange={(e) => setConfirmed(e.target.checked)}
          />{" "}
          I reviewed this file and mapping for {connection.name} ·{" "}
          {table.schema}.{table.name}.
        </label>
      )}
      {status && (
        <section
          className={`import-progress ${status.done ? (status.error ? "failed" : "complete") : ""}`}
          aria-live="polite"
        >
          <strong>
            {status.done
              ? status.error
                ? cancelled
                  ? "Import stopped"
                  : "Import failed"
                : "Import complete"
              : cancelled
                ? "Stopping · waiting for rollback…"
                : "Importing…"}
          </strong>
          <span>
            {status.read_rows.toLocaleString()} records parsed ·{" "}
            {(status.elapsed_ms / 1000).toFixed(1)}s
          </span>
          {!status.done && (
            <small>Parsed records are not committed rows.</small>
          )}
          {status.result && (
            <p>
              {status.result.affected.toLocaleString()} rows inserted
              {status.result.pending_transaction
                ? " · uncommitted: use COMMIT or ROLLBACK."
                : "."}
            </p>
          )}
          {status.error && <p role="alert">{status.error}</p>}
        </section>
      )}
      {error && (
        <p className="form-error" role="alert">
          {error}
        </p>
      )}
      {duplicates && (
        <p className="form-error">Map each destination column only once.</p>
      )}
      {source && !source.preview.rows.length && (
        <p className="form-error">The file has headers but no data records.</p>
      )}
      <footer>
        <button disabled={busy || !!job} onClick={() => void close()}>
          Close
        </button>
        {job ? (
          <button disabled={cancelled} onClick={() => void cancel()}>
            <Square size={13} /> {cancelled ? "Stopping…" : "Cancel import"}
          </button>
        ) : (
          !status && (
            <button
              className="primary"
              onClick={() => void start()}
              disabled={
                busy ||
                !source ||
                !ready ||
                !source.preview.rows.length ||
                !mapped.length ||
                duplicates ||
                (connection.environment === "production" && !confirmed)
              }
            >
              <FileUp size={15} /> Append rows
            </button>
          )
        )}
      </footer>
    </Modal>
  );
}
