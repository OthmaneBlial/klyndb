import { useState } from "react";
import { Bookmark, Trash2 } from "lucide-react";
import type { Connection, History } from "../api";

export interface SavedQuery {
  id: string;
  name: string;
  sql: string;
  connection: string;
  favorite: boolean;
}

type LibraryConnection = Pick<
  Connection,
  "id" | "name" | "engine" | "environment"
>;

function connectionLabel(id: string, connections: LibraryConnection[]) {
  const connection = connections.find((c) => c.id === id);
  return connection
    ? `${connection.name} · ${connection.engine} · ${connection.environment}`
    : `Unavailable connection · ${id}`;
}

function matches(
  id: string,
  text: string,
  connections: LibraryConnection[],
  search: string,
  connection: string,
) {
  return (
    (!connection || id === connection) &&
    `${text}\n${connectionLabel(id, connections)}`
      .toLowerCase()
      .includes(search.trim().toLowerCase())
  );
}

export function filterSavedQueries(
  queries: SavedQuery[],
  connections: LibraryConnection[],
  search: string,
  connection: string,
  favoritesOnly: boolean,
) {
  return queries
    .filter(
      (q) =>
        (!favoritesOnly || q.favorite) &&
        matches(
          q.connection,
          `${q.name}\n${q.sql}`,
          connections,
          search,
          connection,
        ),
    )
    .sort((a, b) => Number(b.favorite) - Number(a.favorite));
}

export function filterQueryHistory(
  history: History[],
  connections: LibraryConnection[],
  search: string,
  connection: string,
  failedOnly: boolean,
) {
  return history.filter(
    (h) =>
      (!failedOnly || h.error !== null) &&
      matches(
        h.connection_id,
        `${h.sql}\n${h.error ?? ""}`,
        connections,
        search,
        connection,
      ),
  );
}

function LibraryFilters({
  search,
  onSearch,
  connection,
  onConnection,
  connections,
  referenced,
}: {
  search: string;
  onSearch: (value: string) => void;
  connection: string;
  onConnection: (value: string) => void;
  connections: LibraryConnection[];
  referenced: string[];
}) {
  const ids = [...new Set([...connections.map((c) => c.id), ...referenced])];
  return (
    <div className="query-library-filters">
      <label>
        Search queries
        <input
          type="search"
          value={search}
          onChange={(e) => onSearch(e.target.value)}
          placeholder="SQL, name or connection…"
        />
      </label>
      <label>
        Connection
        <select
          value={connection}
          onChange={(e) => onConnection(e.target.value)}
        >
          <option value="">All connections</option>
          {ids.map((id) => (
            <option key={id} value={id}>
              {connectionLabel(id, connections)}
            </option>
          ))}
        </select>
      </label>
    </div>
  );
}

export function SavedQueries({
  queries,
  connections,
  onOpen,
  onFavorite,
  onDelete,
}: {
  queries: SavedQuery[];
  connections: LibraryConnection[];
  onOpen: (query: SavedQuery) => void;
  onFavorite: (query: SavedQuery) => void;
  onDelete: (query: SavedQuery) => void;
}) {
  const [search, setSearch] = useState("");
  const [connection, setConnection] = useState("");
  const [favoritesOnly, setFavoritesOnly] = useState(false);
  const shown = filterSavedQueries(
    queries,
    connections,
    search,
    connection,
    favoritesOnly,
  );
  return (
    <>
      <LibraryFilters
        search={search}
        onSearch={setSearch}
        connection={connection}
        onConnection={setConnection}
        connections={connections}
        referenced={queries.map((q) => q.connection)}
      />
      <label className="check">
        <input
          type="checkbox"
          checked={favoritesOnly}
          onChange={(e) => setFavoritesOnly(e.target.checked)}
        />
        Favorites only
      </label>
      <p className="muted" role="status">
        {shown.length} of {queries.length} saved queries · Opening a tab does
        not execute it.
      </p>
      <div className="query-library">
        {shown.map((q) => (
          <div className="saved-query" key={q.id}>
            <button onClick={() => onOpen(q)}>
              <strong>{q.name}</strong>
              <small>{connectionLabel(q.connection, connections)}</small>
              <span>{q.sql}</span>
            </button>
            <button
              className="icon"
              aria-label={`${q.favorite ? "Unfavorite" : "Favorite"} ${q.name}`}
              aria-pressed={q.favorite}
              onClick={() => onFavorite(q)}
            >
              {q.favorite ? "★" : "☆"}
            </button>
            <button
              className="icon"
              aria-label={`Delete saved query ${q.name}`}
              onClick={() => onDelete(q)}
            >
              <Trash2 size={14} />
            </button>
          </div>
        ))}
        {!shown.length && (
          <p className="muted">
            {queries.length
              ? "No saved queries match these filters."
              : "Save a query from the editor with Cmd/Ctrl+S."}
          </p>
        )}
      </div>
    </>
  );
}

export function QueryHistory({
  history,
  connections,
  onOpen,
  onSave,
}: {
  history: History[];
  connections: LibraryConnection[];
  onOpen: (query: History) => void;
  onSave: (query: History) => void;
}) {
  const [search, setSearch] = useState("");
  const [connection, setConnection] = useState("");
  const [failedOnly, setFailedOnly] = useState(false);
  const shown = filterQueryHistory(
    history,
    connections,
    search,
    connection,
    failedOnly,
  );
  return (
    <>
      <LibraryFilters
        search={search}
        onSearch={setSearch}
        connection={connection}
        onConnection={setConnection}
        connections={connections}
        referenced={history.map((h) => h.connection_id)}
      />
      <label className="check">
        <input
          type="checkbox"
          checked={failedOnly}
          onChange={(e) => setFailedOnly(e.target.checked)}
        />
        Failed executions only
      </label>
      <p className="muted" role="status">
        {shown.length} of {history.length} executions · Latest 500 stored
        locally. Opening a tab does not execute it.
      </p>
      <div className="query-library">
        {shown.map((h) => (
          <div className="saved-query" key={h.id}>
            <button onClick={() => onOpen(h)}>
              <small>{connectionLabel(h.connection_id, connections)}</small>
              <span>{h.sql}</span>
              <small>
                {h.created_at} · {h.elapsed_ms} ms
                {h.error !== null ? " · failed" : ""}
              </small>
              {h.error !== null && (
                <span className="query-library-error">{h.error}</span>
              )}
            </button>
            <button
              className="icon"
              aria-label={`Save history query ${h.id}`}
              title="Save this query"
              onClick={() => onSave(h)}
            >
              <Bookmark size={14} />
            </button>
          </div>
        ))}
        {!shown.length && (
          <p className="muted">
            {history.length
              ? "No executions match these filters."
              : "Executed SQL will appear here. History stays on this machine."}
          </p>
        )}
      </div>
    </>
  );
}
