import { Download, KeyRound, Table2 } from "lucide-react";
import type { QueryStatus, Table, TableInfo, TableQuery, Row } from "../api";
import { ResultGrid } from "./ResultGrid";
import { PlanPanel } from "./PlanPanel";
export type ResultView = "results" | "messages" | "structure" | "explain";
export interface Inspector {
  table: Table;
  info: TableInfo;
  query: string;
  browse: TableQuery;
}
function affectedRows(count: number | null | undefined) {
  return count == null
    ? "Affected-row count unavailable"
    : `${count.toLocaleString()} rows affected`;
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
  onLocateError,
  onEdit,
  onDelete,
  editing,
  browsing,
  rowOffset,
  runShortcut = "⌘/Ctrl ↵",
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
  onLocateError?: () => void;
  onEdit?: (row: Row) => void;
  onDelete?: (row: Row) => void;
  editing?: React.ReactNode;
  browsing?: React.ReactNode;
  rowOffset?: number;
  runShortcut?: string;
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
          {status?.plan_format && (
            <button
              className={view === "explain" ? "selected" : ""}
              onClick={() => onView("explain")}
            >
              Explain
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
      {view === "results" && browsing}
      {editing}
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
                  : affectedRows(s.affected)}
              </small>
            </button>
          ))}
        </div>
      )}
      {view === "explain" && status?.plan_format ? (
        <PlanPanel key={`${status.id}-${status.done}`} status={status} />
      ) : view === "structure" && inspector ? (
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
          {inspector.info.statistics && (
            <section aria-label="Table statistics">
              <h4>Statistics</h4>
              <p className="muted">
                {inspector.info.statistics.source}. Rows are planner estimates,
                not live counts. Storage can change while you work.
              </p>
              <table>
                <thead>
                  <tr>
                    <th>Metric</th>
                    <th>Value</th>
                    <th>Unit</th>
                  </tr>
                </thead>
                <tbody>
                  {[
                    [
                      "Rows (estimate)",
                      inspector.info.statistics.estimated_rows,
                      "rows",
                    ],
                    [
                      "Table storage",
                      inspector.info.statistics.table_bytes,
                      "bytes",
                    ],
                    ["Indexes", inspector.info.statistics.index_bytes, "bytes"],
                    [
                      "Total storage",
                      inspector.info.statistics.total_bytes,
                      "bytes",
                    ],
                  ].map(([name, value, unit]) => (
                    <tr key={name}>
                      <td>{name}</td>
                      <td className="statistic-value">
                        {value ?? "Unavailable"}
                      </td>
                      <td>{unit}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </section>
          )}
          <h4>Indexes</h4>
          <pre>{JSON.stringify(inspector.info.indexes, null, 2)}</pre>
          <h4>Foreign keys</h4>
          <pre>{JSON.stringify(inspector.info.foreign_keys, null, 2)}</pre>
          <h4>Constraints</h4>
          {inspector.info.constraints === null ? (
            <p className="muted">
              Constraint definitions are shown in the table DDL below.
            </p>
          ) : inspector.info.constraints.length ? (
            <table>
              <thead>
                <tr>
                  <th>Name</th>
                  <th>Type</th>
                  <th>Definition</th>
                </tr>
              </thead>
              <tbody>
                {inspector.info.constraints.map((c) => (
                  <tr key={c.name}>
                    <td>{c.name}</td>
                    <td>{c.kind}</td>
                    <td>
                      {c.definition ??
                        (inspector.info.ddl
                          ? "See table DDL below"
                          : "Definition unavailable")}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          ) : (
            <p className="muted">No constraints visible to this connection.</p>
          )}
          <h4>Triggers</h4>
          <p className="muted">
            User triggers visible to this connection. Server permissions can
            hide metadata.
          </p>
          {inspector.info.triggers.length ? (
            inspector.info.triggers.map((t) => (
              <details key={t.name}>
                <summary>
                  {t.name}
                  {t.state && <small> · {t.state}</small>}
                </summary>
                <pre>{t.definition}</pre>
              </details>
            ))
          ) : (
            <p className="muted">No user triggers visible.</p>
          )}
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
            <>
              <p className="error">{status.error}</p>
              {onLocateError && (
                <button onClick={onLocateError}>Go to SQL error</button>
              )}
            </>
          ) : status ? (
            <>
              <p>{status.done ? "Query completed." : "Query running…"}</p>
              {status.sets.map((s, i) => (
                <p key={i}>
                  Statement {i + 1}: {s.rows.toLocaleString()} rows returned ·{" "}
                  {affectedRows(s.affected)}
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
          rowOffset={rowOffset}
          onError={onError}
          onEdit={onEdit}
          onDelete={onDelete}
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
              {onLocateError && (
                <button onClick={onLocateError}>Go to SQL error</button>
              )}
            </>
          ) : status ? (
            <>
              <h3>Statement complete</h3>
              <p>{affectedRows(status.sets[set]?.affected)}</p>
            </>
          ) : (
            <>
              <Table2 size={28} />
              <h3>A clear view of your data</h3>
              <p>
                Run a statement or selection
                {runShortcut ? (
                  <>
                    {" "}
                    with <kbd>{runShortcut}</kbd>
                  </>
                ) : (
                  " using the Run button."
                )}
              </p>
            </>
          )}
        </div>
      )}
    </section>
  );
}
