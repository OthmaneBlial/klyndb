import { Download, KeyRound, Table2 } from "lucide-react";
import type { QueryStatus, Table, TableInfo } from "../api";
import { ResultGrid } from "./ResultGrid";
export type ResultView = "results" | "messages" | "structure";
export interface Inspector {
  table: Table;
  info: TableInfo;
}
export function ResultPanel({
  status,
  set,
  onSelectSet,
  view,
  onView,
  inspector,
  busy,
  onError,
  onExport,
}: {
  status?: QueryStatus;
  set: number;
  onSelectSet: (index: number) => void;
  view: ResultView;
  onView: (view: ResultView) => void;
  inspector?: Inspector;
  busy: boolean;
  onError: (message: string) => void;
  onExport: () => void;
}) {
  return (
    <section className="result-area">
      <div className="result-toolbar">
        <div className="result-views">
          <button
            className={view === "results" ? "selected" : ""}
            onClick={() => onView("results")}
          >
            <Table2 size={14} /> Results{" "}
            {status && (
              <small>
                {status.sets.reduce((n, s) => n + s.rows, 0).toLocaleString()}
              </small>
            )}
          </button>
          <button
            className={view === "messages" ? "selected" : ""}
            onClick={() => onView("messages")}
          >
            Messages{status?.error && <span className="error-dot" />}
          </button>
          {inspector && (
            <button
              className={view === "structure" ? "selected" : ""}
              onClick={() => onView("structure")}
            >
              Structure
            </button>
          )}
        </div>
        <div>
          {status?.done && (
            <span className="query-timing">
              {status.elapsed_ms.toLocaleString()} ms
            </span>
          )}
          {status && status.sets[set]?.columns.length > 0 && (
            <button disabled={!status.done} onClick={onExport}>
              <Download size={14} /> Export
            </button>
          )}
        </div>
      </div>
      {status && status.sets.length > 1 && view === "results" && (
        <div className="result-set-tabs">
          {status.sets.map((s, i) => (
            <button
              key={i}
              className={set === i ? "selected" : ""}
              onClick={() => onSelectSet(i)}
            >
              Result {i + 1}
              <small>
                {s.columns.length
                  ? s.rows.toLocaleString()
                  : `${s.affected} affected`}
              </small>
            </button>
          ))}
        </div>
      )}
      {view === "structure" && inspector ? (
        <div className="structure">
          <h3>
            {inspector.table.schema}.{inspector.table.name}
          </h3>
          <table>
            <thead>
              <tr>
                <th>Column</th>
                <th>Type</th>
                <th>Nullable</th>
                <th>Default</th>
              </tr>
            </thead>
            <tbody>
              {inspector.info.columns.map((c) => (
                <tr key={c.name}>
                  <td>
                    {c.primary_key && <KeyRound size={12} />} {c.name}
                  </td>
                  <td>{c.data_type}</td>
                  <td>{c.nullable ? "Yes" : "No"}</td>
                  <td>{c.default ?? "—"}</td>
                </tr>
              ))}
            </tbody>
          </table>
          <h4>Indexes</h4>
          <pre>{JSON.stringify(inspector.info.indexes, null, 2)}</pre>
          <h4>Foreign keys</h4>
          <pre>{JSON.stringify(inspector.info.foreign_keys, null, 2)}</pre>
          {inspector.info.ddl && (
            <>
              <h4>DDL</h4>
              <pre>{inspector.info.ddl}</pre>
            </>
          )}
        </div>
      ) : view === "messages" ? (
        <div className="messages">
          {status?.error ? (
            <p className="error">{status.error}</p>
          ) : status ? (
            <>
              <p>{status.done ? "Query completed." : "Query running…"}</p>
              {status.sets.map((s, i) => (
                <p key={i}>
                  Statement {i + 1}: {s.rows.toLocaleString()} rows returned ·{" "}
                  {s.affected.toLocaleString()} rows affected
                  {s.truncated ? " · row limit reached" : ""}
                </p>
              ))}
            </>
          ) : (
            <p className="muted">No queries executed in this tab yet.</p>
          )}
        </div>
      ) : status?.sets[set]?.columns.length ? (
        <ResultGrid
          key={`${status.id}-${set}`}
          id={status.id}
          set={set}
          metadata={status.sets[set]}
          onError={onError}
        />
      ) : (
        <div className="result-empty">
          {busy ? (
            <>
              <span className="spinner" />
              <h3>Running query</h3>
              <p>Results will appear as they arrive.</p>
            </>
          ) : status?.error ? (
            <>
              <h3>Query failed</h3>
              <p className="error">{status.error}</p>
            </>
          ) : status ? (
            <>
              <h3>Statement complete</h3>
              <p>{status.sets[set]?.affected ?? 0} rows affected</p>
            </>
          ) : (
            <>
              <Table2 size={28} />
              <h3>A clear view of your data</h3>
              <p>
                Run a statement or selection with <kbd>⌘ ↵</kbd>
              </p>
            </>
          )}
        </div>
      )}
    </section>
  );
}
