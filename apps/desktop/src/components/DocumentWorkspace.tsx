import {
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type Ref,
} from "react";
import { Database, Play, Plus, RefreshCw } from "lucide-react";
import {
  api,
  type Connection,
  type DocumentChange,
  type DocumentPage,
  type DocumentQuery,
  type DocumentRecord,
  type Table,
} from "../api";
import { Modal } from "./Modal";
import { documentResultBytes } from "../transientWorkspace";

function JsonTree({
  value,
  label = "document",
}: {
  value: unknown;
  label?: string;
}) {
  const [open, setOpen] = useState(false);
  if (typeof value !== "object" || value === null)
    return (
      <div className="document-leaf">
        <b>{label}</b>: {JSON.stringify(value)}
      </div>
    );
  const entries = Object.entries(value);
  return (
    <details onToggle={(event) => setOpen(event.currentTarget.open)}>
      <summary>
        {label}{" "}
        <small>
          {Array.isArray(value) ? `[${entries.length}]` : `{${entries.length}}`}
        </small>
      </summary>
      {open && (
        <div className="document-tree-children">
          {entries.slice(0, 100).map(([key, child]) => (
            <JsonTree key={key} label={key} value={child} />
          ))}
          {entries.length > 100 && (
            <p className="muted">
              First 100 fields shown; use JSON to see the full document.
            </p>
          )}
        </div>
      )}
    </details>
  );
}
export interface DocumentWorkspaceState {
  kind: "document";
  database: string;
  databases: string[];
  collections: Table[];
  search: string;
  collection: Table | null;
  text: string;
  sort: string;
  aggregate: boolean;
  page: DocumentPage | null;
  applied: DocumentQuery | null;
  selected: DocumentRecord | null;
  indexes: string[] | null;
  tree: boolean;
  error: string;
  message: string;
}
export function clearDocumentResults(
  state: DocumentWorkspaceState,
  reason: string,
): DocumentWorkspaceState {
  return {
    ...state,
    databases: [],
    collections: [],
    page: null,
    applied: null,
    selected: null,
    indexes: null,
    error: "",
    message: reason,
  };
}
export interface DocumentWorkspaceHandle {
  run: () => void;
  refresh: () => void;
}
export function DocumentWorkspace({
  connection,
  ready,
  onBusy,
  workspaceRef,
  initialState,
  onRemember,
  onBackground,
  blocked = false,
}: {
  connection?: Connection;
  ready: boolean;
  onBusy: (value: boolean) => void;
  workspaceRef?: Ref<DocumentWorkspaceHandle>;
  initialState?: DocumentWorkspaceState;
  onRemember?: (
    state: DocumentWorkspaceState,
    bytes: number,
    clear: typeof clearDocumentResults,
  ) => void;
  onBackground?: (patch: Partial<DocumentWorkspaceState>) => void;
  blocked?: boolean;
}) {
  const [state, setState] = useState<DocumentWorkspaceState>(() => {
    if (initialState) return initialState;
    let database = "";
    try {
      database = decodeURIComponent(
        new URL(connection?.address ?? "").pathname.slice(1),
      );
    } catch {
      /* Optional URL database. */
    }
    return {
      kind: "document",
      database,
      databases: [],
      collections: [],
      search: "",
      collection: null,
      text: "{}",
      sort: "{}",
      aggregate: false,
      page: null,
      applied: null,
      selected: null,
      indexes: null,
      tree: false,
      error: "",
      message: "",
    };
  });
  const {
    database,
    databases,
    collections,
    search,
    collection,
    text,
    sort,
    aggregate,
    page,
    applied,
    selected,
    indexes,
    tree,
    error,
    message,
  } = state;
  const [busy, setBusy] = useState(false);
  const [edit, setEdit] = useState<{
    change: DocumentChange;
    review: boolean;
    database: string;
    collection: string;
  } | null>(null);
  // Estimate retained UTF-16 payload once per reply, not on every keystroke.
  const retainedBytes = useMemo(
    () => documentResultBytes({ databases, collections, page, indexes }),
    [databases, collections, page, indexes],
  );
  useEffect(() => {
    onRemember?.(state, retainedBytes, clearDocumentResults);
  }, [onRemember, state, retainedBytes]);
  const pending = useRef(false),
    live = useRef(true);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  function update(patch: Partial<DocumentWorkspaceState>) {
    if (live.current) setState((previous) => ({ ...previous, ...patch }));
    else onBackground?.(patch);
  }
  async function work(task: (id: string) => Promise<void>) {
    if (!connection || !ready || blocked || pending.current) return;
    pending.current = true;
    setBusy(true);
    onBusy(true);
    update({ error: "" });
    try {
      await task(connection.id);
    } catch (error) {
      update({ error: String(error) });
    } finally {
      pending.current = false;
      onBusy(false);
      if (live.current) setBusy(false);
    }
  }
  function loadCollections() {
    void work(async (id) => {
      const result = await api("document_collections", { id, database });
      update({
        collections: result,
        collection: collection
          ? (result.find(
              (item) =>
                item.schema === collection.schema &&
                item.name === collection.name,
            ) ?? null)
          : null,
        page: null,
        selected: null,
        indexes: null,
        applied: null,
      });
    });
  }
  function run(
    query: DocumentQuery = {
      database,
      collection: collection?.name ?? "",
      text,
      sort,
      aggregate,
      offset: 0,
    },
  ) {
    void work(async (id) => {
      const result = await api("document_query", { id, query });
      update({
        page: result,
        applied: query,
        selected: null,
        indexes: null,
        message: "",
      });
    });
  }
  function choose(item: Table) {
    const same =
      collection?.schema === item.schema && collection?.name === item.name;
    update({
      collection: item,
      page: null,
      applied: null,
      selected: null,
      indexes: null,
      ...(same ? {} : { text: "{}", sort: "{}", aggregate: false }),
    });
    run({
      database: item.schema,
      collection: item.name,
      text: same ? text : "{}",
      sort: same ? sort : "{}",
      aggregate: same ? aggregate : false,
      offset: 0,
    });
  }
  useImperativeHandle(workspaceRef, () => ({
    run: () => run(),
    refresh: loadCollections,
  }));
  const disabled = busy || blocked || !ready;
  const writable =
    !!collection &&
    collection.kind === "collection" &&
    collections.some(
      (item) =>
        item.schema === collection.schema &&
        item.name === collection.name &&
        item.kind === "collection",
    ) &&
    !connection?.read_only;
  const editDocument = (change: DocumentChange) =>
    collection &&
    setEdit({
      change,
      review: change.kind === "delete",
      database: collection.schema,
      collection: collection.name,
    });
  async function apply() {
    if (!edit) return;
    const reviewed = edit;
    setEdit(null);
    await work(async (id) => {
      const result = await api("document_change", {
        id,
        database: reviewed.database,
        collection: reviewed.collection,
        change: reviewed.change,
        confirmed: true,
      });
      update({
        message: `${result.affected} document${result.affected === 1 ? "" : "s"} changed. Write acknowledged.`,
        selected: null,
        page: null,
      });
      if (applied) {
        try {
          const result = await api("document_query", { id, query: applied });
          update({ page: result });
        } catch (error) {
          throw new Error(`Write acknowledged, but refresh failed. ${error}`);
        }
      }
    });
  }
  return (
    <section
      className="key-workspace document-workspace"
      aria-label="MongoDB document workspace"
    >
      <aside className="key-catalog">
        <header>
          <Database size={16} />
          <strong>Collections</strong>
          <span className="eyebrow">MONGO</span>
        </header>
        <div className="key-search">
          <label>
            Database
            <input
              aria-label="MongoDB database"
              value={database}
              list="mongo-databases"
              maxLength={255}
              disabled={busy || blocked}
              onChange={(e) => {
                update({
                  database: e.target.value,
                  collections: [],
                  collection: null,
                  page: null,
                  selected: null,
                  applied: null,
                  indexes: null,
                });
              }}
            />
          </label>
          <datalist id="mongo-databases">
            {databases.map((name) => (
              <option key={name} value={name} />
            ))}
          </datalist>
          <div className="document-actions">
            <button
              disabled={disabled}
              onClick={() =>
                void work(async (id) => {
                  const result = await api("document_databases", { id });
                  update({ databases: result });
                })
              }
            >
              List databases
            </button>
            <button disabled={disabled || !database} onClick={loadCollections}>
              <RefreshCw size={13} /> Load collections
            </button>
          </div>
          <input
            aria-label="Filter collections"
            placeholder="Filter collections…"
            value={search}
            onChange={(e) => update({ search: e.target.value })}
          />
        </div>
        <div className="key-list">
          {collections
            .filter((item) =>
              item.name.toLowerCase().includes(search.toLowerCase()),
            )
            .map((item) => (
              <button
                key={item.name}
                className={`key-item ${collection?.name === item.name ? "selected" : ""}`}
                disabled={disabled}
                onClick={() => choose(item)}
              >
                <span className="key-name">{item.name}</span>
                <small>{item.kind}</small>
              </button>
            ))}
          {!collections.length && (
            <p className="key-hint muted">
              Load a database’s collections when you need them. You can enter a
              database name directly if listing databases is restricted.
            </p>
          )}
        </div>
        <footer>{collections.length} collections · loaded on demand</footer>
      </aside>
      <div className="key-content">
        {!ready && (
          <p className="key-hint muted">
            Connect to MongoDB to browse documents.
          </p>
        )}
        {error && (
          <div className="notice" role="alert">
            {error}
          </div>
        )}
        {message && (
          <p className="notice" role="status">
            {message}
          </p>
        )}
        <section
          className="key-console document-query"
          aria-label="Document query"
        >
          <header>
            <div>
              <span className="eyebrow">
                {collection?.schema ?? "DATABASE"}
              </span>
              <h2>{collection?.name ?? "Choose a collection"}</h2>
            </div>
            <button
              className="primary"
              disabled={disabled || !collection}
              onClick={() => run()}
            >
              <Play size={13} /> Run <kbd>⌘ ↵</kbd>
            </button>
          </header>
          <div className="document-actions">
            <label>
              Mode
              <select
                aria-label="Document query mode"
                disabled={busy || blocked}
                value={aggregate ? "aggregate" : "find"}
                onChange={(e) => {
                  const next = e.target.value === "aggregate";
                  update({ aggregate: next, text: next ? "[]" : "{}" });
                }}
              >
                <option value="find">Find documents</option>
                <option value="aggregate">Aggregation pipeline</option>
              </select>
            </label>
            {!aggregate && (
              <label>
                Sort · 1 ascending, -1 descending
                <input
                  aria-label="Document sort"
                  spellCheck={false}
                  autoCorrect="off"
                  autoCapitalize="off"
                  value={sort}
                  maxLength={16384}
                  disabled={busy || blocked}
                  onChange={(e) => update({ sort: e.target.value })}
                />
              </label>
            )}
          </div>
          <label>
            {aggregate ? "Pipeline · JSON array" : "Filter · JSON object"}
            <textarea
              aria-label={
                aggregate ? "Aggregation pipeline" : "Document filter"
              }
              value={text}
              maxLength={1024 * 1024}
              spellCheck={false}
              disabled={busy || blocked}
              onChange={(e) => update({ text: e.target.value })}
              onKeyDown={(e) => {
                if ((e.metaKey || e.ctrlKey) && e.key === "Enter") {
                  e.preventDefault();
                  if (collection) run();
                }
              }}
            />
          </label>
          <p className="muted">
            Extended JSON preserves ObjectId, dates, binary, decimals and 64-bit
            integers. Read-only pipelines; no $out, $merge or server-side
            JavaScript. Drafts stay in memory.
          </p>
        </section>
        <section className="key-inspection" aria-label="Documents and indexes">
          <header>
            <h2>
              {indexes ? "Indexes" : "Documents"}{" "}
              {page && !indexes && (
                <small>{page.documents.length} in this page</small>
              )}
            </h2>
            <div className="document-actions">
              <button
                disabled={disabled || !collection}
                onClick={() =>
                  void work(async (id) => {
                    const result = await api("document_indexes", {
                      id,
                      database: collection!.schema,
                      collection: collection!.name,
                    });
                    update({ indexes: result });
                  })
                }
              >
                Indexes
              </button>
              <button
                disabled={disabled || !page}
                onClick={() => update({ indexes: null })}
              >
                Documents
              </button>
              <button
                disabled={disabled || !writable}
                onClick={() => editDocument({ kind: "insert", json: "{}" })}
              >
                <Plus size={13} /> Insert document
              </button>
            </div>
          </header>
          {busy && (
            <p role="status">
              Waiting for MongoDB… requests have a 10-second deadline.
            </p>
          )}
          {indexes ? (
            indexes.map((index, i) => (
              <pre className="key-reply" key={i} tabIndex={0}>
                {index}
              </pre>
            ))
          ) : (
            <>
              <div className="document-list">
                {page?.documents.map((row, index) => (
                  <button
                    key={index}
                    className={`document-card ${row === selected ? "selected" : ""}`}
                    onClick={() => update({ selected: row })}
                    disabled={disabled}
                  >
                    <small>Document {(applied?.offset ?? 0) + index + 1}</small>
                    <pre>
                      {row.json.slice(0, 1000)}
                      {row.json.length > 1000 ? "\n…" : ""}
                    </pre>
                  </button>
                ))}
              </div>
              {page && !page.documents.length && (
                <p className="key-hint muted">No matching documents.</p>
              )}
              <div className="key-paging">
                <span>
                  Offset {applied?.offset ?? 0} · 100 documents per page
                </span>
                <button
                  disabled={disabled || !applied?.offset}
                  onClick={() =>
                    applied &&
                    run({
                      ...applied,
                      offset: Math.max(0, applied.offset - 100),
                    })
                  }
                >
                  Previous page
                </button>
                <button
                  disabled={disabled || !page?.has_more || !applied}
                  onClick={() =>
                    applied && run({ ...applied, offset: applied.offset + 100 })
                  }
                >
                  Next page
                </button>
              </div>
            </>
          )}
          {selected && !indexes && (
            <section
              className="document-inspector"
              aria-label="Selected document"
            >
              <div className="document-actions">
                <button
                  onClick={() => update({ tree: false })}
                  aria-pressed={!tree}
                >
                  JSON
                </button>
                <button
                  onClick={() => update({ tree: true })}
                  aria-pressed={tree}
                >
                  Tree
                </button>
                <button
                  disabled={disabled || !writable || !selected.snapshot}
                  onClick={() =>
                    selected.snapshot &&
                    editDocument({
                      kind: "replace",
                      snapshot: selected.snapshot,
                      json: selected.json,
                    })
                  }
                >
                  Edit document
                </button>
                <button
                  className="danger"
                  disabled={disabled || !writable || !selected.snapshot}
                  onClick={() =>
                    selected.snapshot &&
                    editDocument({
                      kind: "delete",
                      snapshot: selected.snapshot,
                    })
                  }
                >
                  Delete document
                </button>
              </div>
              {tree ? (
                <div className="document-tree">
                  <JsonTree value={JSON.parse(selected.json)} />
                </div>
              ) : (
                <pre className="key-reply" tabIndex={0}>
                  {selected.json}
                </pre>
              )}
              {!selected.snapshot && (
                <p className="muted">
                  Aggregation results are read-only. Find the original document
                  to edit it.
                </p>
              )}
            </section>
          )}
        </section>
      </div>
      {edit && (
        <Modal
          title={
            edit.review
              ? `${edit.change.kind === "delete" ? "Delete" : "Apply"} document${connection?.environment === "production" ? " in production" : ""}?`
              : "Edit document"
          }
          onClose={() => setEdit(null)}
          wide
        >
          <p>
            {edit.database}.{edit.collection} · {connection?.name} ·{" "}
            {connection?.environment}
          </p>
          {edit.review ? (
            <>
              <p>
                One atomic, acknowledged write. Replacement and deletion check
                the original document for concurrent changes. No automatic
                retries.
              </p>
              <pre className="key-reply">
                {"json" in edit.change ? edit.change.json : selected?.json}
              </pre>
            </>
          ) : (
            <label>
              Document · Extended JSON
              <textarea
                className="document-edit"
                aria-label="Document edit JSON"
                spellCheck={false}
                value={"json" in edit.change ? edit.change.json : ""}
                maxLength={1024 * 1024}
                onChange={(e) => {
                  const json = e.target.value;
                  setEdit((current) =>
                    current && "json" in current.change
                      ? { ...current, change: { ...current.change, json } }
                      : current,
                  );
                }}
              />
            </label>
          )}
          <div className="modal-actions">
            <button onClick={() => setEdit(null)}>Cancel</button>
            {edit.review ? (
              <button className="danger" onClick={() => void apply()}>
                Confirm {edit.change.kind === "delete" ? "delete" : "write"}
              </button>
            ) : (
              <button
                className="primary"
                onClick={() => setEdit({ ...edit, review: true })}
              >
                Review change
              </button>
            )}
          </div>
        </Modal>
      )}
    </section>
  );
}
