import { useCallback, useEffect, useRef, useState } from "react";
import { api, type Routine, type RoutinePage } from "../api";
import { Modal } from "./Modal";

export function RoutineDefinition({
  routine,
  definition,
  onOpen,
}: {
  routine: Routine;
  definition: string;
  onOpen: () => void;
}) {
  return (
    <section className="routine-definition" aria-label="Routine definition">
      <div className="routine-definition-heading">
        <div>
          <strong>
            {routine.schema}.{routine.name}({routine.arguments})
          </strong>
          <small>
            {routine.kind} · {routine.language}
            {routine.returns ? ` → ${routine.returns}` : ""}
          </small>
        </div>
        <button className="primary" onClick={onOpen}>
          Open in SQL tab
        </button>
      </div>
      <p className="muted">
        Server-provided routine definition. Opening a tab does not execute it.
      </p>
      <pre tabIndex={0}>{definition}</pre>
    </section>
  );
}

export function RoutineBrowser({
  connection,
  onClose,
  onOpen,
}: {
  connection: { id: string; name: string };
  onClose: () => void;
  onOpen: (routine: Routine, definition: string) => void;
}) {
  const [search, setSearch] = useState("");
  const [page, setPage] = useState<RoutinePage | null>(null);
  const [offset, setOffset] = useState(0);
  const [applied, setApplied] = useState("");
  const [busy, setBusy] = useState(false);
  const [selected, setSelected] = useState<Routine | null>(null);
  const [definition, setDefinition] = useState<string | null>(null);
  const [error, setError] = useState("");
  const requests = useRef({ listing: 0, inspection: 0 });
  const load = useCallback(
    async (query: string, start: number) => {
      const request = ++requests.current.listing;
      ++requests.current.inspection;
      setBusy(true);
      setError("");
      setSelected(null);
      setDefinition(null);
      setPage(null);
      try {
        const next = await api("routines", {
          id: connection.id,
          search: query,
          offset: start,
        });
        if (request !== requests.current.listing) return;
        setPage(next);
        setOffset(start);
        setApplied(query);
      } catch (e) {
        if (request === requests.current.listing) setError(String(e));
      } finally {
        if (request === requests.current.listing) setBusy(false);
      }
    },
    [connection.id],
  );
  useEffect(() => {
    const pending = requests.current;
    void load("", 0);
    return () => {
      ++pending.listing;
      ++pending.inspection;
    };
  }, [load]);
  async function inspect(routine: Routine) {
    const request = ++requests.current.inspection;
    setSelected(routine);
    setDefinition(null);
    setError("");
    try {
      const text = await api("routine_definition", {
        id: connection.id,
        routineId: routine.id,
      });
      if (request === requests.current.inspection) setDefinition(text);
    } catch (e) {
      if (request === requests.current.inspection) setError(String(e));
    }
  }
  return (
    <Modal
      title={`Functions & procedures · ${connection.name}`}
      onClose={onClose}
      wide
      className="routine-modal"
    >
      <form
        className="routine-search"
        onSubmit={(e) => {
          e.preventDefault();
          void load(search, 0);
        }}
      >
        <label>
          Search schema or routine name
          <input
            value={search}
            maxLength={1024}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="analytics.calculate_total"
          />
        </label>
        <button type="submit" disabled={busy}>
          Search
        </button>
        <button
          type="button"
          disabled={busy}
          onClick={() => void load(applied, offset)}
        >
          Refresh
        </button>
      </form>
      {error && (
        <p role="alert" className="error-message">
          {error}
        </p>
      )}
      <div className="routine-layout">
        <section
          className="routine-catalog"
          aria-label="Routine catalog"
          aria-busy={busy}
        >
          {busy && (
            <p className="muted" role="status">
              Loading routines…
            </p>
          )}
          {page?.routines.map((routine) => (
            <button
              key={routine.id}
              aria-pressed={selected?.id === routine.id}
              onClick={() => void inspect(routine)}
            >
              <strong>
                {routine.schema}.{routine.name}
              </strong>
              <code>({routine.arguments})</code>
              <small>
                {routine.kind}
                {routine.returns ? ` → ${routine.returns}` : ""}
              </small>
            </button>
          ))}
          {page && !page.routines.length && (
            <p className="muted">
              No matching functions or procedures visible.
            </p>
          )}
        </section>
        <div className="routine-detail">
          {selected ? (
            definition !== null ? (
              <RoutineDefinition
                routine={selected}
                definition={definition}
                onOpen={() => onOpen(selected, definition)}
              />
            ) : (
              !error && (
                <p className="muted" role="status">
                  Loading definition…
                </p>
              )
            )
          ) : (
            <p className="muted">
              Select a function or procedure to inspect its native definition.
              Overloads are listed separately.
            </p>
          )}
        </div>
      </div>
      <footer>
        <small>
          {page &&
            `${offset + (page.routines.length ? 1 : 0)}–${offset + page.routines.length}${page.has_more ? " · more available" : ""}`}
        </small>
        <button
          disabled={busy || !page || offset === 0}
          onClick={() => void load(applied, Math.max(0, offset - 100))}
        >
          Previous
        </button>
        <button
          disabled={busy || !page?.has_more}
          onClick={() => void load(applied, offset + 100)}
        >
          Next
        </button>
        <button onClick={onClose}>Done</button>
      </footer>
    </Modal>
  );
}
