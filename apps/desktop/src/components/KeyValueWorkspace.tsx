import {
  useEffect,
  useImperativeHandle,
  useMemo,
  useRef,
  useState,
  type Ref,
} from "react";
import { KeyRound, Play, RefreshCw, Search } from "lucide-react";
import {
  api,
  type Connection,
  type KeyCommandInfo,
  type KeyInspection,
  type KeyScan,
  type KeyValue,
  type Cell,
} from "../api";
import { formatKeyValue, keyLabel, ttlLabel } from "../keyValue";
import { Modal } from "./Modal";

export function KeyReply({ value }: { value: KeyValue }) {
  return (
    <pre className="key-reply" tabIndex={0}>
      {formatKeyValue(value)}
    </pre>
  );
}
export interface KeyWorkspaceState {
  kind: "key_value";
  pattern: string;
  page: KeyScan | null;
  applied: string;
  selected: Cell | null;
  inspection: KeyInspection | null;
  positions: string[];
  reply: KeyValue | null;
  error: string;
  message: string;
}
export function clearKeyResults(
  state: KeyWorkspaceState,
  reason: string,
): KeyWorkspaceState {
  return {
    ...state,
    page: null,
    selected: null,
    inspection: null,
    positions: [],
    reply: null,
    error: "",
    message: reason,
  };
}
export interface KeyWorkspaceHandle {
  run: () => void;
  scan: () => void;
}
export function KeyValueWorkspace({
  connection,
  ready,
  draft,
  onDraft,
  onBusy,
  workspaceRef,
  initialState,
  onRemember,
  blocked = false,
}: {
  connection?: Connection;
  ready: boolean;
  draft: string;
  onDraft: (text: string) => void;
  onBusy: (busy: boolean) => void;
  workspaceRef?: Ref<KeyWorkspaceHandle>;
  initialState?: KeyWorkspaceState;
  onRemember?: (
    state: KeyWorkspaceState,
    bytes: number,
    clear: typeof clearKeyResults,
  ) => void;
  blocked?: boolean;
}) {
  const [pattern, setPattern] = useState(initialState?.pattern ?? "*");
  const [page, setPage] = useState<KeyScan | null>(initialState?.page ?? null);
  const [applied, setApplied] = useState(initialState?.applied ?? "*");
  const [selected, setSelected] = useState<Cell | null>(
    initialState?.selected ?? null,
  );
  const [inspection, setInspection] = useState<KeyInspection | null>(
    initialState?.inspection ?? null,
  );
  const [positions, setPositions] = useState<string[]>(
    initialState?.positions ?? [],
  );
  const [reply, setReply] = useState<KeyValue | null>(
    initialState?.reply ?? null,
  );
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(initialState?.error ?? "");
  const [message, setMessage] = useState(initialState?.message ?? "");
  const [confirm, setConfirm] = useState<{
    text: string;
    info: KeyCommandInfo;
  } | null>(null);
  const retainedBytes = useMemo(
    () => JSON.stringify({ page, inspection, reply, positions }).length * 2,
    [page, inspection, reply, positions],
  );
  useEffect(() => {
    onRemember?.(
      {
        kind: "key_value",
        pattern,
        page,
        applied,
        selected,
        inspection,
        positions,
        reply,
        error,
        message,
      },
      retainedBytes,
      clearKeyResults,
    );
  }, [
    onRemember,
    pattern,
    page,
    applied,
    selected,
    inspection,
    positions,
    reply,
    error,
    message,
    retainedBytes,
  ]);
  const live = useRef(true),
    pending = useRef(false);
  useEffect(() => {
    live.current = true;
    return () => {
      live.current = false;
    };
  }, []);
  async function work(task: (id: string) => Promise<void>) {
    if (!connection || !ready || blocked || pending.current) return;
    pending.current = true;
    setBusy(true);
    onBusy(true);
    setError("");
    try {
      await task(connection.id);
    } catch (error) {
      if (live.current) setError(String(error));
    } finally {
      pending.current = false;
      onBusy(false);
      if (live.current) setBusy(false);
    }
  }
  function scan(cursor: string, query: string) {
    void work(async (id) => {
      const result = await api("scan_keys", { id, pattern: query, cursor });
      if (live.current) {
        setPage(result);
        setApplied(query);
        setMessage("");
      }
    });
  }
  function inspect(key: Cell, position: string, previous: string[]) {
    void work(async (id) => {
      const result = await api("inspect_key", { id, key, position });
      if (live.current) {
        setSelected(key);
        setInspection(result);
        setPositions(previous);
        setMessage("");
      }
    });
  }
  async function execute(text: string, confirmed = false) {
    await work(async (id) => {
      const info = await api("key_command_info", { id, text });
      if (connection?.read_only && info.writes)
        throw new Error("This Redis connection is read-only");
      if (!live.current) return;
      if (
        connection?.environment === "production" &&
        info.writes &&
        !confirmed
      ) {
        setConfirm({ text, info });
        return;
      }
      const result = await api("key_command", { id, text, confirmed });
      if (live.current) {
        setReply(result);
        setMessage("");
        setInspection(null);
      }
    });
  }
  const disabled = !ready || busy || blocked;
  useImperativeHandle(workspaceRef, () => ({
    run: () => {
      void execute(draft);
    },
    scan: () => scan("0", pattern),
  }));
  return (
    <section className="key-workspace" aria-label="Redis key workspace">
      <aside className="key-catalog">
        <header>
          <KeyRound size={16} />
          <strong>Key explorer</strong>
          <span className="eyebrow">REDIS</span>
        </header>
        <form
          className="key-search"
          onSubmit={(event) => {
            event.preventDefault();
            scan("0", pattern);
          }}
        >
          <label>
            Search pattern
            <input
              aria-label="Key search pattern"
              value={pattern}
              maxLength={1024}
              onChange={(event) => setPattern(event.target.value)}
              placeholder="cache:*"
              disabled={busy || blocked}
            />
          </label>
          <button className="primary" disabled={disabled}>
            <Search size={14} /> Scan keys
          </button>
        </form>
        <div className="key-list" aria-label="Keys">
          {page?.keys.map((entry, index) => (
            <button
              key={`${JSON.stringify(entry.key)}-${index}`}
              className={`key-item ${selected && JSON.stringify(selected) === JSON.stringify(entry.key) ? "selected" : ""}`}
              disabled={disabled}
              onClick={() => inspect(entry.key, "", [])}
            >
              <span className="key-name" title={keyLabel(entry.key)}>
                {keyLabel(entry.key)}
              </span>
              <span className="key-details">
                <b>{entry.data_type}</b>
                <small>{ttlLabel(entry.ttl_ms)}</small>
              </span>
            </button>
          ))}
          {!page && (
            <p className="muted key-hint">
              Scan when you need the keyspace. No eager database-wide load.
            </p>
          )}
          {page && !page.keys.length && (
            <p className="muted key-hint">
              No keys in this scan step.
              {page.cursor !== "0"
                ? " Continue scanning to search the remaining keyspace."
                : " Scan finished."}
            </p>
          )}
        </div>
        <footer>
          <span>
            {page
              ? `${page.keys.length} keys in this step`
              : "Native cursor paging"}
          </span>
          <button
            disabled={disabled || !page || page.cursor === "0"}
            onClick={() => page && scan(page.cursor, applied)}
          >
            Next scan step
          </button>
          <small>
            Cursor {page?.cursor ?? "0"} · COUNT is a hint; live keys may move
            or repeat.
          </small>
        </footer>
      </aside>
      <div className="key-content">
        {!ready && (
          <p className="key-hint muted">
            Connect to Redis to inspect keys and run native commands.
          </p>
        )}
        {message && (
          <p className="notice" role="status">
            {message}
          </p>
        )}
        {error && (
          <div className="notice" role="alert">
            {error}
          </div>
        )}
        <section className="key-inspection" aria-label="Key inspection">
          <header>
            <div>
              <span className="eyebrow">VALUE INSPECTOR</span>
              <h2>{selected ? keyLabel(selected) : "Select a key"}</h2>
            </div>
            <button
              disabled={disabled || !selected}
              onClick={() => selected && inspect(selected, "", [])}
            >
              <RefreshCw size={14} /> Refresh value
            </button>
          </header>
          {inspection ? (
            <>
              <div className="key-metadata">
                <span>
                  <small>TYPE</small>
                  <b>{inspection.entry.data_type}</b>
                </span>
                <span>
                  <small>TTL</small>
                  <b>{ttlLabel(inspection.entry.ttl_ms)}</b>
                </span>
                <span>
                  <small>
                    {inspection.entry.data_type === "string"
                      ? "BYTES"
                      : "ITEMS"}
                  </small>
                  <b>{inspection.length}</b>
                </span>
              </div>
              <KeyReply value={inspection.value} />
              <div className="key-paging">
                <span className="muted">
                  Position {inspection.position || "0"} ·{" "}
                  {inspection.entry.data_type === "string"
                    ? "64 KiB byte ranges"
                    : inspection.entry.data_type === "list" ||
                        inspection.entry.data_type === "stream"
                      ? "100-item ranges"
                      : "Native scan cursor"}
                </span>
                <button
                  disabled={disabled || !positions.length}
                  onClick={() =>
                    selected &&
                    inspect(
                      selected,
                      positions[positions.length - 1],
                      positions.slice(0, -1),
                    )
                  }
                >
                  Previous value page
                </button>
                <button
                  disabled={disabled || !inspection.next}
                  onClick={() =>
                    selected &&
                    inspection.next &&
                    inspect(selected, inspection.next, [
                      ...positions,
                      inspection.position,
                    ])
                  }
                >
                  Next value page
                </button>
              </div>
            </>
          ) : (
            <p className="muted key-hint">
              Types, TTLs and original values. Text and hexadecimal bytes stay
              lossless; integers stay exact.
            </p>
          )}
        </section>
        <section
          className="key-console"
          aria-label="Native Redis command console"
        >
          <header>
            <div>
              <span className="eyebrow">NATIVE COMMANDS</span>
              <h2>Talk to your cache.</h2>
            </div>
            <button
              className="primary"
              disabled={disabled}
              onClick={() => void execute(draft)}
            >
              <Play size={13} /> Run command <kbd>⌘ ↵</kbd>
            </button>
          </header>
          <label>
            JSON argument array
            <textarea
              aria-label="Redis command arguments"
              value={draft}
              onChange={(event) => onDraft(event.target.value)}
              maxLength={256 * 1024}
              spellCheck={false}
              disabled={busy || blocked}
              onKeyDown={(event) => {
                if ((event.metaKey || event.ctrlKey) && event.key === "Enter") {
                  event.preventDefault();
                  void execute(draft);
                }
              }}
            />
          </label>
          <p className="muted">
            Examples: ["GET", "cache:key"] · ["HSET", "profile:42", "name",
            "Ada"]. Data commands only. No scripts, blocking calls or session
            changes. Drafts remain in memory.
          </p>
          {busy && (
            <p role="status">
              Waiting for Redis… requests time out after 10 seconds.
            </p>
          )}
          {reply && (
            <div aria-label="Command reply">
              <KeyReply value={reply} />
            </div>
          )}
        </section>
      </div>
      {confirm && (
        <Modal
          title={`Run ${confirm.info.command} in production?`}
          onClose={() => setConfirm(null)}
        >
          <p>
            This command can change keys on {connection?.name}. Check the exact
            arguments before running it.
          </p>
          <pre className="key-reply">{confirm.text}</pre>
          <div className="modal-actions">
            <button onClick={() => setConfirm(null)}>Cancel</button>
            <button
              className="danger"
              onClick={() => {
                const text = confirm.text;
                setConfirm(null);
                void execute(text, true);
              }}
            >
              Confirm write
            </button>
          </div>
        </Modal>
      )}
    </section>
  );
}
