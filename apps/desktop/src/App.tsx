import {
  useState,
  useEffect,
  useRef,
  useCallback,
  useMemo,
  lazy,
  Suspense,
} from "react";
import {
  Database,
  Plus,
  Search,
  Play,
  Square,
  Settings,
  PanelLeft,
  ChevronRight,
  ChevronDown,
  Table2,
  RefreshCw,
  Unplug,
  Pencil,
  Copy,
  Trash2,
  Clock,
  Bookmark,
  X,
  WandSparkles,
  Download,
  FileUp,
  Shield,
  Command,
  ArrowUpRight,
  FileCode2,
  Network,
} from "lucide-react";

import {
  api,
  type Connection,
  type Table,
  type QueryStatus,
  type History,
  type Capabilities,
  type Change,
  type Row,
  type TableQuery,
} from "./api";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { ConnectionDialog } from "./components/ConnectionDialog";
import type { EditorHandle } from "./components/SqlEditor";
const SqlEditor = lazy(() =>
  import("./components/SqlEditor").then((m) => ({ default: m.SqlEditor })),
);
import {
  ResultPanel,
  type ResultView,
  type Inspector,
} from "./components/ResultPanel";
import { CommandPalette } from "./components/CommandPalette";
import { SettingsDialog } from "./components/SettingsDialog";
import { RowDialog } from "./components/RowDialog";
import { TableControls } from "./components/TableControls";
import { ImportDialog } from "./components/ImportDialog";
import { SqlImportDialog } from "./components/SqlImportDialog";
import { Modal } from "./components/Modal";
const DiagramDialog = lazy(() =>
  import("./components/DiagramDialog").then((m) => ({
    default: m.DiagramDialog,
  })),
);
import {
  defaults,
  restoreWorkspace,
  type Preferences,
  type Tab,
} from "./workspace";

interface SavedQuery {
  id: string;
  name: string;
  sql: string;
  connection: string;
  favorite: boolean;
}
export default function App() {
  const [connections, setConnections] = useState<Connection[]>([]),
    [connected, setConnected] = useState<Record<string, Capabilities>>({}),
    [tables, setTables] = useState<Record<string, Table[]>>({}),
    [columns, setColumns] = useState<Record<string, Record<string, string[]>>>(
      {},
    );
  const [tabs, setTabs] = useState<Tab[]>([]),
    [active, setActive] = useState(""),
    [preferences, setPreferences] = useState<Preferences>(defaults),
    [loaded, setLoaded] = useState(false),
    [notice, setNotice] = useState(""),
    [connecting, setConnecting] = useState<string[]>([]);
  const [dialog, setDialog] = useState<Connection | true | null>(null),
    [settings, setSettings] = useState(false),
    [diagramConnection, setDiagramConnection] = useState<Connection | null>(
      null,
    ),
    [palette, setPalette] = useState(false),
    [sidebarSearch, setSidebarSearch] = useState(""),
    [expanded, setExpanded] = useState<Record<string, boolean>>({});
  const [statuses, setStatuses] = useState<Record<string, QueryStatus>>({}),
    [resultSets, setResultSets] = useState<Record<string, number>>({}),
    [view, setView] = useState<ResultView>("results"),
    [inspectors, setInspectors] = useState<Record<string, Inspector>>({});
  const [history, setHistory] = useState<History[] | null>(null),
    [saved, setSaved] = useState<SavedQuery[]>([]),
    [savedOpen, setSavedOpen] = useState(false),
    [saveName, setSaveName] = useState<string | null>(null),
    [confirm, setConfirm] = useState<{
      title: string;
      message: string;
      sql?: string;
      action: () => void;
    } | null>(null),
    [exportOpen, setExportOpen] = useState(false),
    [exportFormat, setExportFormat] = useState("csv");
  const [staged, setStaged] = useState<Record<string, Change[]>>({}),
    [tableJobs, setTableJobs] = useState<Record<string, string>>({}),
    [reviewChanges, setReviewChanges] = useState(false),
    [rowDialog, setRowDialog] = useState<{
      tab: Tab;
      inspector: Inspector;
      old: Row | null;
    } | null>(null),
    [importDialog, setImportDialog] = useState<{
      tab: Tab;
      inspector: Inspector;
      connection: Connection;
    } | null>(null),
    [sqlImport, setSqlImport] = useState<Connection | null>(null),
    [applying, setApplying] = useState<Record<string, boolean>>({}),
    [transactionStates, setTransactionStates] = useState<
      Record<string, "idle" | "active" | "failed" | "unknown">
    >({});
  const applyingRef = useRef(applying);
  applyingRef.current = applying;
  const importDialogRef = useRef(importDialog);
  importDialogRef.current = importDialog;
  const sqlImportRef = useRef(sqlImport);
  sqlImportRef.current = sqlImport;
  const diagramRef = useRef(diagramConnection);
  diagramRef.current = diagramConnection;
  const stagedRef = useRef(staged);
  stagedRef.current = staged;
  const editorRef = useRef<EditorHandle | null>(null);
  const browsingRef = useRef(new Set<string>());
  const [browsing, setBrowsing] = useState<Record<string, boolean>>({});
  const current = tabs.find((t) => t.id === active),
    connection = connections.find((c) => c.id === current?.connection),
    status = statuses[active],
    inspector = inspectors[active],
    busy =
      (!!status && !status.done) || !!applying[active] || !!browsing[active],
    set = resultSets[active] ?? 0;
  const editable = !!(
    connection &&
    connected[connection.id]?.edit_rows &&
    !connection.read_only &&
    inspector?.info.editable &&
    tableJobs[active] === status?.id &&
    status?.done &&
    !status.error &&
    set === 0 &&
    ["table", "base table"].includes(inspector.table.kind) &&
    status.sets[0]?.columns.every(
      (name, i) => name === inspector.info.columns[i]?.name,
    ) &&
    status.sets[0]?.columns.length === inspector.info.columns.length
  );
  const tableResult = !!(
    inspector &&
    status &&
    tableJobs[active] === status.id &&
    set === 0
  );
  const keyed = !!inspector?.info.columns.some((c) => c.primary_key);
  const report = useCallback((message: string) => setNotice(message), []);
  const updateTab = useCallback(
    (id: string, patch: Partial<Tab>) =>
      setTabs((t) =>
        t.map((tab) => (tab.id === id ? { ...tab, ...patch } : tab)),
      ),
    [],
  );
  const schema = useMemo(
    () =>
      Object.fromEntries(
        (tables[current?.connection ?? ""] ?? []).map((t) => [
          t.name,
          columns[current?.connection ?? ""]?.[`${t.schema}.${t.name}`] ?? [],
        ]),
      ),
    [tables, columns, current?.connection],
  );
  useEffect(() => {
    Promise.all([
      api("connections"),
      api("load_document", { id: "workspace" }),
      api("load_document", { id: "saved-queries" }),
    ])
      .then(([connections, workspace, queries]) => {
        setConnections(connections);
        const restored = restoreWorkspace(workspace);
        setTabs(restored.tabs);
        setActive(restored.active);
        setPreferences(restored.preferences);
        if (Array.isArray(queries))
          setSaved(
            queries.filter(
              (q) =>
                q &&
                typeof q.name === "string" &&
                typeof q.sql === "string" &&
                typeof q.connection === "string" &&
                typeof q.id === "string",
            ),
          );
        setLoaded(true);
      })
      .catch((e) => report(`Could not load local state: ${e}`));
  }, [report]);
  const workspaceRef = useRef({ version: 1, tabs, active, preferences });
  workspaceRef.current = { version: 1, tabs, active, preferences };
  const loadedRef = useRef(loaded);
  loadedRef.current = loaded;
  const saveQueue = useRef(Promise.resolve());
  const persistWorkspace = useCallback(() => {
    saveQueue.current = saveQueue.current
      .catch(() => {})
      .then(() =>
        api("save_document", { id: "workspace", data: workspaceRef.current }),
      );
    return saveQueue.current;
  }, []);
  useEffect(() => {
    if (!loaded) return;
    const timer = setTimeout(() => {
      persistWorkspace().catch((e) =>
        report(`Workspace could not be saved: ${e}`),
      );
    }, 400);
    return () => clearTimeout(timer);
  }, [tabs, active, preferences, loaded, persistWorkspace, report]);
  useEffect(() => {
    let dispose: (() => void) | undefined;
    let live = true;
    getCurrentWindow()
      .onCloseRequested(async (event) => {
        event.preventDefault();
        if (diagramRef.current) {
          report(
            "Close the diagram to save its layout before closing the workspace.",
          );
          return;
        }
        if (importDialogRef.current || sqlImportRef.current) {
          report("Close the import dialog before closing the workspace.");
          return;
        }
        if (Object.values(applyingRef.current).some(Boolean)) {
          report("Wait for the editing batch to finish before closing.");
          return;
        }
        const close = async () => {
          try {
            if (loadedRef.current) await persistWorkspace();
            await getCurrentWindow().destroy();
          } catch (e) {
            report(`Could not save before closing: ${e}`);
          }
        };
        if (
          Object.values(stagedRef.current).some((changes) => changes.length)
        ) {
          setConfirm({
            title: "Discard staged changes and close?",
            message:
              "Staged edits have not been applied. SQL tabs and workspace will be saved.",
            action: () => void close(),
          });
        } else await close();
      })
      .then((unlisten) => {
        if (live) dispose = unlisten;
        else unlisten();
      })
      .catch((e) => report(String(e)));
    return () => {
      live = false;
      dispose?.();
    };
  }, [persistWorkspace, report]);
  useEffect(() => {
    document.documentElement.dataset.theme = preferences.theme;
    document.documentElement.style.setProperty(
      "--editor-font-size",
      `${preferences.fontSize}px`,
    );
  }, [preferences]);
  const running = Object.entries(statuses)
    .filter(([, s]) => !s.done)
    .map(([tab, s]) => `${tab}:${s.id}`)
    .join("|");
  useEffect(() => {
    let live = true;
    const timer = setInterval(() => {
      for (const item of running.split("|").filter(Boolean)) {
        const [tab, id] = item.split(":");
        api("query_status", { id })
          .then((status) => {
            if (live && status.done && status.transaction)
              setTransactionStates((s) => ({
                ...s,
                [status.connection_id]: status.transaction!,
              }));
            if (live)
              setStatuses((s) =>
                s[tab]?.id === id ? { ...s, [tab]: status } : s,
              );
          })
          .catch((e) => {
            if (live) {
              report(String(e));
              setStatuses((s) =>
                s[tab]
                  ? { ...s, [tab]: { ...s[tab], done: true, error: String(e) } }
                  : s,
              );
            }
          });
      }
    }, 180);
    return () => {
      live = false;
      clearInterval(timer);
    };
  }, [running, report]);
  function newTab(
    connectionId = connection?.id ?? connections[0]?.id ?? "",
    sql = "SELECT 1;",
    name?: string,
  ) {
    if (tabs.length >= 100) {
      report("The workspace is limited to 100 tabs. Close an unused tab.");
      return;
    }
    const id = crypto.randomUUID();
    const tab: Tab = {
      id,
      name: name ?? `Query ${tabs.length + 1}`,
      connection: connectionId,
      sql,
    };
    setTabs((t) => [...t, tab]);
    setActive(id);
    setView("results");
    return tab;
  }
  async function closeTab(id: string, discard = false) {
    if (applyingRef.current[id] || browsingRef.current.has(id)) {
      report("Wait for the current table operation to finish.");
      return;
    }
    if (!discard && stagedRef.current[id]?.length) {
      setConfirm({
        title: "Discard staged changes?",
        message: "These changes have not been applied to the database.",
        action: () => void closeTab(id, true).catch((e) => report(String(e))),
      });
      return;
    }
    setStaged((s) => {
      const next = { ...s };
      delete next[id];
      return next;
    });
    const result = statuses[id];
    if (result) await api("release_result", { id: result.id });
    setStatuses((s) => {
      const next = { ...s };
      delete next[id];
      return next;
    });
    setInspectors((s) => {
      const next = { ...s };
      delete next[id];
      return next;
    });
    setTabs((t) => t.filter((tab) => tab.id !== id));
    if (active === id) setActive(tabs.find((t) => t.id !== id)?.id ?? "");
  }
  async function switchTabConnection(id: string, connection: string) {
    if (browsingRef.current.has(id)) return;
    if (stagedRef.current[id]?.length) {
      report("Apply or discard staged changes before switching connection.");
      return;
    }
    if (statuses[id]) await api("release_result", { id: statuses[id].id });
    setStatuses((s) => {
      const next = { ...s };
      delete next[id];
      return next;
    });
    setInspectors((s) => {
      const next = { ...s };
      delete next[id];
      return next;
    });
    updateTab(id, { connection });
    setView("results");
  }
  async function refresh(id: string) {
    setTables((t) => ({ ...t, [id]: [] }));
    try {
      const result = await api("tables", { id });
      setTables((t) => ({ ...t, [id]: result }));
    } catch (e) {
      report(String(e));
    }
  }
  async function connect(
    c: Connection,
    password: string | null = null,
    identityPassword: string | null = null,
    sshPassword: string | null = null,
  ) {
    if (connecting.includes(c.id)) return;
    setConnecting((ids) => [...ids, c.id]);
    try {
      const capabilities = await api("connect", {
        id: c.id,
        password,
        identityPassword,
        sshPassword,
      });
      setConnected((s) => ({ ...s, [c.id]: capabilities }));
      setExpanded((s) => ({ ...s, [c.id]: true }));
      await refresh(c.id);
      if (!tabs.some((t) => t.connection === c.id)) newTab(c.id);
      else setActive(tabs.find((t) => t.connection === c.id)!.id);
      setNotice("");
    } catch (e) {
      report(String(e));
    } finally {
      setConnecting((ids) => ids.filter((id) => id !== c.id));
    }
  }
  async function disconnect(c: Connection) {
    try {
      await api("disconnect", { id: c.id });
      setConnected((s) => {
        const next = { ...s };
        delete next[c.id];
        return next;
      });
      setTables((t) => {
        const next = { ...t };
        delete next[c.id];
        return next;
      });
    } catch (e) {
      report(String(e));
    }
  }
  const starting = useRef(new Set<string>());
  async function run(
    sql = editorRef.current?.runText() ?? current?.sql ?? "",
    confirmed = false,
    plan?: "estimate" | "analyze",
  ) {
    if (browsingRef.current.has(active)) return;
    if (stagedRef.current[active]?.length) {
      report("Apply or discard staged changes before running another query.");
      return;
    }
    if (current)
      await executeTab(current, sql, confirmed, inspectors[current.id], plan);
  }
  async function executeTab(
    tab: Tab,
    sql: string,
    confirmed = false,
    inspected = inspectors[tab.id],
    plan?: "estimate" | "analyze",
  ) {
    const c = connections.find((c) => c.id === tab.connection),
      previous = statuses[tab.id];
    if (!c || starting.current.has(tab.id) || (previous && !previous.done))
      return;
    starting.current.add(tab.id);
    setNotice("");
    try {
      if (!connected[c.id]) {
        report("Connect to this database before running SQL.");
        return;
      }
      const warnings =
        plan === "estimate"
          ? []
          : (await api("analyze_query", { sql, engine: c.engine })).warnings;
      if (plan === "analyze")
        warnings.unshift(
          "ANALYZE executes this statement, including writes and side effects. It does not roll back automatically.",
        );
      if (!confirmed && warnings.length) {
        setConfirm({
          title: plan === "analyze" ? "Run ANALYZE?" : "Confirm SQL execution",
          message: `${c.name} · ${c.environment}\n${warnings.join("\n")}`,
          sql,
          action: () => {
            void executeTab(tab, sql, true, inspected, plan);
          },
        });
        return;
      }
      const id = plan
        ? await api("start_plan", {
            connection: c.id,
            sql,
            analyze: plan === "analyze",
            timeoutSeconds: preferences.timeout,
            confirmed,
          })
        : await api("start_query", {
            connection: c.id,
            sql,
            limit:
              inspected?.query === sql
                ? inspected.browse.limit
                : preferences.rowLimit,
            timeoutSeconds: preferences.timeout,
            confirmed,
          });
      const initial = await api("query_status", { id });
      if (previous) await api("release_result", { id: previous.id });
      setStatuses((s) => ({
        ...s,
        [tab.id]: initial,
      }));
      setTableJobs((s) => {
        const next = { ...s };
        if (!plan && inspected?.query === sql) next[tab.id] = id;
        else delete next[tab.id];
        return next;
      });
      setResultSets((s) => ({ ...s, [tab.id]: 0 }));
      setView(plan ? "explain" : "results");
      return true;
    } catch (e) {
      report(String(e));
    } finally {
      starting.current.delete(tab.id);
    }
  }
  async function browseTable(
    tab: Tab,
    inspected: Inspector,
    query: TableQuery,
  ) {
    if (
      browsingRef.current.has(tab.id) ||
      starting.current.has(tab.id) ||
      applyingRef.current[tab.id] ||
      statuses[tab.id]?.done === false
    )
      return;
    if (stagedRef.current[tab.id]?.length) {
      report("Apply or discard staged changes before changing the table page.");
      return;
    }
    browsingRef.current.add(tab.id);
    setBrowsing((s) => ({ ...s, [tab.id]: true }));
    try {
      const sql = await api("table_query_sql", {
        id: tab.connection,
        table: inspected.table,
        query,
      });
      const next = { ...inspected, query: sql, browse: query };
      if (await executeTab(tab, sql, false, next)) {
        setInspectors((s) => ({ ...s, [tab.id]: next }));
        updateTab(tab.id, { sql });
      }
    } catch (e) {
      report(String(e));
    } finally {
      browsingRef.current.delete(tab.id);
      setBrowsing((s) => ({ ...s, [tab.id]: false }));
    }
  }
  async function openTable(c: Connection, table: Table) {
    try {
      const info = await api("inspect_table", { id: c.id, table });
      setColumns((s) => ({
        ...s,
        [c.id]: {
          ...s[c.id],
          [`${table.schema}.${table.name}`]: info.columns.map((c) => c.name),
        },
      }));
      const browse: TableQuery = {
        filters: [],
        sort: [],
        limit: 500,
        offset: 0,
      };
      const query = await api("table_query_sql", {
        id: c.id,
        table,
        query: browse,
      });
      const tab = newTab(c.id, query, table.name);
      if (tab) {
        const inspected = { table, info, query: tab.sql, browse };
        setInspectors((s) => ({ ...s, [tab.id]: inspected }));
        await executeTab(tab, tab.sql, false, inspected);
      }
    } catch (e) {
      report(String(e));
    }
  }
  function stage(tab: string, change: Change) {
    setStaged((s) => {
      const changes = s[tab] ?? [];
      const old = change.kind === "insert" ? null : JSON.stringify(change.old);
      return {
        ...s,
        [tab]: [
          ...changes.filter(
            (c) => !old || c.kind === "insert" || JSON.stringify(c.old) !== old,
          ),
          change,
        ],
      };
    });
  }
  async function applyStaged(
    tab: Tab,
    inspected: Inspector,
    confirmed = false,
  ) {
    const changes = stagedRef.current[tab.id] ?? [],
      c = connections.find((c) => c.id === tab.connection);
    if (!changes.length || !c || starting.current.has(tab.id)) return;
    if (
      !confirmed &&
      (c.environment === "production" ||
        changes.some((change) => change.kind === "delete"))
    ) {
      setConfirm({
        title: "Confirm table changes",
        message: `${c.name} · ${c.environment}: apply ${changes.length} staged changes to ${inspected.table.schema}.${inspected.table.name}?`,
        action: () => void applyStaged(tab, inspected, true),
      });
      return;
    }
    starting.current.add(tab.id);
    setApplying((s) => ({ ...s, [tab.id]: true }));
    try {
      const result = await api("apply_changes", {
        id: c.id,
        table: inspected.table,
        changes,
        confirmed,
      });
      setStaged((s) => ({ ...s, [tab.id]: [] }));
      setTransactionStates((s) => ({
        ...s,
        [c.id]: result.pending_transaction ? "active" : "idle",
      }));
      starting.current.delete(tab.id);
      await executeTab(tab, inspected.query, false, inspected);
      report(
        `${result.affected} changes applied${result.pending_transaction ? " · uncommitted transaction: use COMMIT or ROLLBACK in the editor." : "."}`,
      );
    } catch (e) {
      report(String(e));
    } finally {
      // Errors such as a server deadlock can roll back the user's whole transaction.
      const state = await api("transaction_state", { id: c.id }).catch(
        () => "unknown" as const,
      );
      setTransactionStates((s) => ({ ...s, [c.id]: state }));
      starting.current.delete(tab.id);
      setApplying((s) => ({ ...s, [tab.id]: false }));
    }
  }
  async function formatSql() {
    try {
      const { format } = await import("sql-formatter");
      editorRef.current?.replace(
        format(editorRef.current.allText(), {
          language:
            connection?.engine === "postgres"
              ? "postgresql"
              : connection?.engine === "mysql"
                ? "mysql"
                : "sqlite",
        }),
      );
    } catch (e) {
      report(`Could not format SQL: ${e}`);
    }
  }
  async function duplicate(c: Connection) {
    try {
      const copy = await api("save_connection", {
        connection: { ...c, id: "", name: `${c.name} copy` },
        password: null,
        remember: true,
      });
      setConnections((s) => [...s, copy]);
    } catch (e) {
      report(String(e));
    }
  }
  function deleteConnection(c: Connection) {
    setConfirm({
      title: "Delete saved connection",
      message: `Remove ${c.name} and its stored credentials? The database itself is preserved.`,
      action: () => {
        api("delete_connection", { id: c.id })
          .then(() => {
            setConnections((s) => s.filter((item) => item.id !== c.id));
            setConnected((s) => {
              const next = { ...s };
              delete next[c.id];
              return next;
            });
          })
          .catch((e) => report(String(e)));
      },
    });
  }
  async function saveQuery() {
    if (!current || !saveName?.trim()) return;
    const queries = [
      ...saved,
      {
        id: crypto.randomUUID(),
        name: saveName.trim(),
        sql: current.sql,
        connection: current.connection,
        favorite: false,
      },
    ];
    try {
      await api("save_document", { id: "saved-queries", data: queries });
      setSaved(queries);
      setSaveName(null);
    } catch (e) {
      report(String(e));
    }
  }
  async function changeSaved(next: SavedQuery[]) {
    try {
      await api("save_document", { id: "saved-queries", data: next });
      setSaved(next);
    } catch (e) {
      report(String(e));
    }
  }
  const commands = [
    { name: "New SQL tab", key: "⌘ T", action: () => newTab() },
    { name: "New connection", key: "", action: () => setDialog(true) },
    {
      name: "Run current statement or selection",
      key: "⌘ ↵",
      action: () => void run(),
    },
    {
      name: "Run entire editor",
      key: "",
      action: () => void run(editorRef.current?.allText()),
    },
    { name: "Format SQL", key: "⇧ ⌘ F", action: formatSql },
    ...(connection && connected[connection.id]?.import_sql
      ? [
          {
            name: "Import SQL file",
            key: "",
            action: () => setSqlImport(connection),
          },
        ]
      : []),
    {
      name: "Refresh schema",
      key: "",
      action: () => connection && void refresh(connection.id),
    },
    {
      name: "Open relationship diagram",
      key: "",
      action: () => {
        if (connection && connected[connection.id]?.diagrams)
          setDiagramConnection(connection);
        else
          report(
            "Connect to a supported database and select its SQL tab first.",
          );
      },
    },
    { name: "Saved queries", key: "", action: () => setSavedOpen(true) },
    {
      name: "Query history",
      key: "",
      action: () =>
        void api("history")
          .then(setHistory)
          .catch((e) => report(String(e))),
    },
    {
      name: "Toggle sidebar",
      key: "",
      action: () => setPreferences((p) => ({ ...p, sidebar: !p.sidebar })),
    },
    { name: "Settings", key: "", action: () => setSettings(true) },
    ...connections.map((c) => ({
      name: `Connect · ${c.name}`,
      key: "",
      action: () => void connect(c),
    })),
    ...(tables[current?.connection ?? ""] ?? []).map((t) => ({
      name: `Open table · ${t.schema}.${t.name}`,
      key: "",
      action: () => connection && void openTable(connection, t),
    })),
  ];
  useEffect(() => {
    const key = (e: KeyboardEvent) => {
      if (importDialogRef.current || sqlImportRef.current || diagramRef.current)
        return;
      if (!(e.metaKey || e.ctrlKey)) return;
      if (e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPalette((p) => !p);
      }
      if (e.key.toLowerCase() === "t") {
        e.preventDefault();
        newTab();
      }
      if (e.key.toLowerCase() === "s") {
        e.preventDefault();
        setSaveName(current?.name ?? "");
      }
      if (e.shiftKey && e.key.toLowerCase() === "f") {
        e.preventDefault();
        formatSql();
      }
    };
    window.addEventListener("keydown", key);
    return () => window.removeEventListener("keydown", key);
  });
  const groups = [...new Set(connections.map((c) => c.group || "Personal"))];
  return (
    <div className={`workbench ${preferences.sidebar ? "" : "sidebar-hidden"}`}>
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark">k</span>
          <strong>Klyndb</strong>
          <span className="preview-label">PREVIEW</span>
        </div>
        <div className="topbar-center">
          {connection ? (
            <>
              <span
                className="connection-dot"
                style={{ background: connection.color }}
              />
              <span>{connection.name}</span>
              <span className={`environment ${connection.environment}`}>
                {connection.environment}
              </span>
              {connection.read_only && <Shield size={13} />}
            </>
          ) : (
            <span className="muted">Your databases. Your workspace.</span>
          )}
        </div>
        <button
          className="palette-trigger"
          onClick={() => {
            setPalette(true);
          }}
        >
          <Search size={14} />
          <span>Jump to anything</span>
          <kbd>⌘ K</kbd>
        </button>
        <button
          className="icon"
          aria-label="Settings"
          onClick={() => setSettings(true)}
        >
          <Settings size={17} />
        </button>
      </header>
      {preferences.sidebar && (
        <aside className="sidebar">
          <div className="sidebar-heading">
            <span>WORKSPACE</span>
            <button
              className="icon"
              aria-label="New connection"
              onClick={() => setDialog(true)}
            >
              <Plus size={16} />
            </button>
          </div>
          <div className="sidebar-search search">
            <Search size={14} />
            <input
              aria-label="Search connections and tables"
              placeholder="Find a connection or table…"
              value={sidebarSearch}
              onChange={(e) => setSidebarSearch(e.target.value)}
            />
          </div>
          <div className="connection-tree">
            {groups.map((group) => (
              <section key={group}>
                <h3>{group}</h3>
                {connections
                  .filter((c) => (c.group || "Personal") === group)
                  .map((c) => {
                    const allTables = tables[c.id] ?? [],
                      matches = allTables.filter((t) =>
                        `${t.schema}.${t.name}`
                          .toLowerCase()
                          .includes(sidebarSearch.toLowerCase()),
                      );
                    if (
                      sidebarSearch &&
                      !c.name
                        .toLowerCase()
                        .includes(sidebarSearch.toLowerCase()) &&
                      !matches.length
                    )
                      return null;
                    return (
                      <div key={c.id} className="connection-node">
                        <div
                          className={`connection-item ${current?.connection === c.id ? "active" : ""}`}
                        >
                          <button
                            className="connection-name"
                            onClick={() =>
                              connected[c.id]
                                ? setExpanded((s) => ({
                                    ...s,
                                    [c.id]: !s[c.id],
                                  }))
                                : void connect(c)
                            }
                          >
                            {expanded[c.id] ? (
                              <ChevronDown size={13} />
                            ) : (
                              <ChevronRight size={13} />
                            )}
                            <Database size={16} style={{ color: c.color }} />
                            <span>
                              {c.favorite ? "★ " : ""}
                              {c.name}
                            </span>
                            <span
                              className={`status-dot ${connected[c.id] ? "online" : ""}`}
                            />
                          </button>
                        </div>
                        <div className="connection-actions">
                          <span className={`environment ${c.environment}`}>
                            {connecting.includes(c.id)
                              ? "connecting…"
                              : c.environment}
                          </span>
                          <button
                            className="icon"
                            aria-label={`Edit ${c.name}`}
                            onClick={() => setDialog(c)}
                          >
                            <Pencil size={12} />
                          </button>
                          <button
                            className="icon"
                            aria-label={`Duplicate ${c.name}`}
                            onClick={() => void duplicate(c)}
                          >
                            <Copy size={12} />
                          </button>
                          {connected[c.id] ? (
                            <button
                              className="icon"
                              aria-label={`Disconnect ${c.name}`}
                              onClick={() => void disconnect(c)}
                            >
                              <Unplug size={12} />
                            </button>
                          ) : (
                            <button
                              className="icon"
                              aria-label={`Delete ${c.name}`}
                              onClick={() => deleteConnection(c)}
                            >
                              <Trash2 size={12} />
                            </button>
                          )}
                        </div>
                        {expanded[c.id] && connected[c.id] && (
                          <div className="tables-list">
                            {connected[c.id]?.diagrams && (
                              <button
                                className="diagram-open"
                                onClick={() => setDiagramConnection(c)}
                              >
                                <Network size={13} /> Relationships
                              </button>
                            )}
                            <div className="tables-heading">
                              <span>
                                Tables & views <small>{allTables.length}</small>
                              </span>
                              <button
                                className="icon"
                                aria-label={`Refresh ${c.name} schema`}
                                onClick={() => void refresh(c.id)}
                              >
                                <RefreshCw size={12} />
                              </button>
                            </div>
                            {matches.map((table) => (
                              <button
                                className="table-item"
                                key={`${table.schema}.${table.name}`}
                                onClick={() => void openTable(c, table)}
                              >
                                <Table2 size={13} />
                                <span>
                                  {table.schema !== "main"
                                    ? `${table.schema}.`
                                    : ""}
                                  {table.name}
                                </span>
                              </button>
                            ))}
                            {!allTables.length && (
                              <p className="muted tree-empty">
                                No tables. Create one in a SQL tab.
                              </p>
                            )}
                          </div>
                        )}
                      </div>
                    );
                  })}
              </section>
            ))}
          </div>
          <div className="sidebar-footer">
            <button
              onClick={() =>
                void api("history")
                  .then(setHistory)
                  .catch((e) => report(String(e)))
              }
            >
              <Clock size={15} /> Query history
            </button>
            <button onClick={() => setSavedOpen(true)}>
              <Bookmark size={15} /> Saved queries
              <span>{saved.length || ""}</span>
            </button>
            <button onClick={() => setDialog(true)}>
              <Plus size={15} /> Add connection
            </button>
            <div className="local-note">
              <Shield size={12} /> Local-first · no account
            </div>
          </div>
        </aside>
      )}
      <main className="main">
        <div className="tabbar">
          <button
            className="icon toggle-sidebar"
            aria-label="Toggle sidebar"
            onClick={() =>
              setPreferences((p) => ({ ...p, sidebar: !p.sidebar }))
            }
          >
            <PanelLeft size={16} />
          </button>
          <div className="tabs">
            {tabs.map((tab) => (
              <div
                key={tab.id}
                className={`tab ${tab.id === active ? "active" : ""}`}
              >
                <button
                  onClick={() => {
                    setActive(tab.id);
                    setView("results");
                  }}
                >
                  <FileCode2 size={13} />
                  <span>{tab.name}</span>
                  <small>
                    {connections.find((c) => c.id === tab.connection)?.name ??
                      "No connection"}
                  </small>
                  {statuses[tab.id] && !statuses[tab.id].done && (
                    <span className="running-dot" />
                  )}
                </button>
                <button
                  className="tab-close"
                  aria-label={`Close ${tab.name}`}
                  onClick={() =>
                    void closeTab(tab.id).catch((e) => report(String(e)))
                  }
                >
                  <X size={12} />
                </button>
              </div>
            ))}
          </div>
          <button
            className="icon"
            aria-label="New SQL tab"
            onClick={() => newTab()}
          >
            <Plus size={17} />
          </button>
        </div>
        {notice && (
          <div className="notice" role="alert">
            <span>{notice}</span>
            <button
              className="icon"
              aria-label="Dismiss message"
              onClick={() => setNotice("")}
            >
              <X size={14} />
            </button>
          </div>
        )}
        {current ? (
          <>
            <div className="query-toolbar">
              <div>
                <select
                  aria-label="Tab connection"
                  value={current.connection}
                  disabled={busy}
                  onChange={(e) =>
                    void switchTabConnection(current.id, e.target.value).catch(
                      (e) => report(String(e)),
                    )
                  }
                >
                  <option value="">Choose connection</option>
                  {connections.map((c) => (
                    <option key={c.id} value={c.id}>
                      {c.name}
                    </option>
                  ))}
                </select>
                {connection && !connected[connection.id] && (
                  <button onClick={() => void connect(connection)}>
                    <Database size={14} /> Connect
                  </button>
                )}
              </div>
              <div>
                <button
                  title="Format SQL · Shift+Cmd/Ctrl+F"
                  onClick={formatSql}
                >
                  <WandSparkles size={14} /> Format
                </button>
                <button
                  title="Save query · Cmd/Ctrl+S"
                  onClick={() => setSaveName(current.name)}
                >
                  <Bookmark size={14} />
                </button>
                {applying[active] ? (
                  <span className="muted">Applying batch…</span>
                ) : busy ? (
                  <button
                    className="danger"
                    onClick={() =>
                      void api("cancel_query", { id: status.id }).catch((e) =>
                        report(String(e)),
                      )
                    }
                  >
                    <Square size={13} /> Cancel
                  </button>
                ) : (
                  <>
                    {connection && connected[connection.id]?.import_sql && (
                      <button onClick={() => setSqlImport(connection)}>
                        <FileUp size={14} /> Import SQL
                      </button>
                    )}
                    {connection && connected[connection.id]?.explain && (
                      <>
                        <button
                          title="Estimate the current statement or selection without executing it"
                          onClick={() => void run(undefined, false, "estimate")}
                        >
                          Explain
                        </button>
                        {connected[connection.id]?.explain_analyze && (
                          <button
                            title="Execute with runtime statistics; confirmation required"
                            onClick={() =>
                              void run(undefined, false, "analyze")
                            }
                          >
                            Analyze
                          </button>
                        )}
                      </>
                    )}
                    <button
                      className="primary"
                      disabled={!connection || !connected[connection.id]}
                      onClick={() => void run()}
                    >
                      <Play size={13} fill="currentColor" /> Run <kbd>⌘ ↵</kbd>
                    </button>
                    <button
                      title="Run all statements"
                      disabled={!connection || !connected[connection.id]}
                      onClick={() => void run(editorRef.current?.allText())}
                    >
                      All
                    </button>
                  </>
                )}
              </div>
            </div>
            <section className="editor-area">
              <Suspense
                fallback={
                  <div className="result-empty">Loading SQL editor…</div>
                }
              >
                <SqlEditor
                  key={current.id}
                  value={current.sql}
                  engine={connection?.engine ?? "sqlite"}
                  schema={schema}
                  onChange={(sql) => updateTab(current.id, { sql })}
                  onRun={(sql) => void run(sql)}
                  editorRef={editorRef}
                />
              </Suspense>
            </section>
            <ResultPanel
              status={status}
              set={set}
              onSelectSet={(i) => setResultSets((p) => ({ ...p, [active]: i }))}
              view={view}
              onView={setView}
              inspector={inspector}
              busy={busy}
              onError={report}
              onExport={() => setExportOpen(true)}
              onEdit={
                editable && keyed && !busy
                  ? (old) => setRowDialog({ tab: current, inspector, old })
                  : undefined
              }
              onDelete={
                editable && keyed && !busy
                  ? (old) => stage(active, { kind: "delete", old })
                  : undefined
              }
              rowOffset={tableResult ? inspector?.browse.offset : undefined}
              browsing={
                inspector &&
                connection &&
                connected[connection.id]?.table_browse && (
                  <TableControls
                    key={`${current.id}-${inspector.query}`}
                    columns={inspector.info.columns}
                    query={inspector.browse}
                    active={tableResult}
                    busy={busy}
                    staged={!!staged[active]?.length}
                    done={!!status?.done}
                    failed={!!status?.error}
                    rows={status?.sets[0]?.rows ?? 0}
                    onBrowse={(query) => browseTable(current, inspector, query)}
                  />
                )
              }
              editing={
                editable && (
                  <div className="table-editing">
                    <button
                      disabled={busy}
                      onClick={() =>
                        setRowDialog({ tab: current, inspector, old: null })
                      }
                    >
                      <Plus size={14} /> Insert row
                    </button>
                    {connection && connected[connection.id]?.import_rows && (
                      <button
                        disabled={
                          busy ||
                          !!staged[active]?.length ||
                          Object.values(statuses).some(
                            (s) => s.connection_id === connection.id && !s.done,
                          )
                        }
                        onClick={() =>
                          setImportDialog({
                            tab: current,
                            inspector,
                            connection,
                          })
                        }
                      >
                        <FileUp size={14} /> Import data
                      </button>
                    )}
                    <span className="muted">
                      {staged[active]?.length ?? 0} staged changes
                      {!keyed ? " · no primary key: insert only" : ""}
                    </span>
                    {!!staged[active]?.length && (
                      <>
                        <button
                          disabled={busy}
                          onClick={() => setReviewChanges(true)}
                        >
                          Review
                        </button>
                        <button
                          disabled={busy}
                          onClick={() =>
                            setStaged((s) => ({ ...s, [active]: [] }))
                          }
                        >
                          Discard
                        </button>
                        <button
                          className="primary"
                          disabled={busy}
                          onClick={() => void applyStaged(current, inspector)}
                        >
                          Apply batch
                        </button>
                      </>
                    )}
                  </div>
                )
              }
            />
            <footer className="statusbar">
              <span>
                <span
                  className={`status-dot ${connection && connected[connection.id] ? "online" : ""}`}
                />
                {connection && connected[connection.id]
                  ? `${connection.engine} · connected`
                  : "Disconnected"}
                {connection?.read_only ? " · read-only" : ""}
                {connection &&
                transactionStates[connection.id] !== undefined &&
                transactionStates[connection.id] !== "idle"
                  ? transactionStates[connection.id] === "unknown"
                    ? " · transaction state unavailable (check connection)"
                    : ` · transaction ${transactionStates[connection!.id]}${transactionStates[connection!.id] === "failed" ? " (ROLLBACK required)" : " (COMMIT or ROLLBACK)"}`
                  : ""}
              </span>
              <span>
                Limit {preferences.rowLimit.toLocaleString()} · Timeout{" "}
                {preferences.timeout}s
              </span>
              <span>
                {busy ? "Executing…" : status?.done ? "Ready" : "SQL workbench"}
              </span>
            </footer>
          </>
        ) : (
          <div className="welcome">
            <div className="welcome-symbol">
              <Database size={36} strokeWidth={1.2} />
            </div>
            <span className="eyebrow">A QUIETER DATABASE WORKSPACE</span>
            <h1>
              Make room for
              <br />
              <em>your data.</em>
            </h1>
            <p>
              A native Rust core. A focused SQL workbench.
              <br />
              Everything stays on your machine.
            </p>
            <div className="welcome-actions">
              <button className="primary" onClick={() => setDialog(true)}>
                <Plus size={16} /> Add a connection <ArrowUpRight size={16} />
              </button>
              {connections.length > 0 && (
                <button onClick={() => newTab()}>Open SQL tab</button>
              )}
            </div>
            <div className="welcome-engines">
              <span>SQLite</span>
              <span>PostgreSQL</span>
              <small>More drivers in development</small>
            </div>
            <div className="welcome-shortcut">
              <Command size={13} /> K{" "}
              <span>Search commands, connections and tables</span>
            </div>
          </div>
        )}
      </main>
      {dialog && (
        <ConnectionDialog
          initial={dialog === true ? undefined : dialog}
          onClose={() => setDialog(null)}
          onSaved={(
            c,
            password,
            shouldConnect,
            identityPassword,
            sshPassword,
          ) => {
            setConnections((s) => [...s.filter((item) => item.id !== c.id), c]);
            setConnected((s) => {
              const next = { ...s };
              delete next[c.id];
              return next;
            });
            if (shouldConnect)
              void connect(c, password, identityPassword, sshPassword);
          }}
        />
      )}
      {confirm && (
        <Modal title={confirm.title} onClose={() => setConfirm(null)}>
          <p className="confirmation-text">{confirm.message}</p>
          {confirm.sql && <pre className="confirmation-sql">{confirm.sql}</pre>}
          <footer>
            <button onClick={() => setConfirm(null)}>Cancel</button>
            <button
              className="danger"
              onClick={() => {
                confirm.action();
                setConfirm(null);
              }}
            >
              Confirm
            </button>
          </footer>
        </Modal>
      )}
      {settings && (
        <SettingsDialog
          preferences={preferences}
          setPreferences={setPreferences}
          onClose={() => setSettings(false)}
        />
      )}
      {palette && (
        <CommandPalette commands={commands} onClose={() => setPalette(false)} />
      )}
      {history && (
        <Modal title="Query history" onClose={() => setHistory(null)} wide>
          <div className="query-library">
            {history.map((h) => (
              <button
                key={h.id}
                onClick={() => {
                  newTab(h.connection_id, h.sql);
                  setHistory(null);
                }}
              >
                <span>{h.sql}</span>
                <small>
                  {h.created_at} · {h.elapsed_ms} ms{h.error ? " · failed" : ""}
                </small>
              </button>
            ))}
            {!history.length && (
              <p className="muted">
                Executed SQL will appear here. History stays on this machine.
              </p>
            )}
          </div>
          <footer>
            <button
              className="danger"
              onClick={() =>
                setConfirm({
                  title: "Clear query history",
                  message: "Remove the locally stored SQL history?",
                  action: () => {
                    api("clear_history")
                      .then(() => setHistory([]))
                      .catch((e) => report(String(e)));
                  },
                })
              }
            >
              Clear history
            </button>
          </footer>
        </Modal>
      )}
      {savedOpen && (
        <Modal title="Saved queries" onClose={() => setSavedOpen(false)} wide>
          <div className="query-library">
            {[...saved]
              .sort((a, b) => Number(b.favorite) - Number(a.favorite))
              .map((q) => (
                <div className="saved-query" key={q.id}>
                  <button
                    onClick={() => {
                      newTab(q.connection, q.sql, q.name);
                      setSavedOpen(false);
                    }}
                  >
                    <strong>{q.name}</strong>
                    <span>{q.sql}</span>
                  </button>
                  <button
                    className="icon"
                    aria-label={`Favorite ${q.name}`}
                    onClick={() =>
                      void changeSaved(
                        saved.map((s) =>
                          s.id === q.id ? { ...s, favorite: !s.favorite } : s,
                        ),
                      )
                    }
                  >
                    {q.favorite ? "★" : "☆"}
                  </button>
                  <button
                    className="icon"
                    aria-label={`Delete saved query ${q.name}`}
                    onClick={() =>
                      setConfirm({
                        title: "Delete saved query",
                        message: `Remove ${q.name}?`,
                        action: () =>
                          void changeSaved(saved.filter((s) => s.id !== q.id)),
                      })
                    }
                  >
                    <Trash2 size={14} />
                  </button>
                </div>
              ))}
            {!saved.length && (
              <p className="muted">
                Save a query from the editor with Cmd/Ctrl+S.
              </p>
            )}
          </div>
        </Modal>
      )}
      {saveName !== null && (
        <Modal title="Save query" onClose={() => setSaveName(null)}>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void saveQuery();
            }}
          >
            <label>
              Query name
              <input
                autoFocus
                value={saveName}
                onChange={(e) => setSaveName(e.target.value)}
                required
              />
            </label>
            <footer>
              <button className="primary">Save query</button>
            </footer>
          </form>
        </Modal>
      )}
      {reviewChanges && (
        <Modal
          title="Staged changes"
          onClose={() => setReviewChanges(false)}
          wide
        >
          {(staged[active] ?? []).map((change, i) => (
            <div className="staged-change" key={i}>
              <strong>
                {i + 1}. {change.kind}
              </strong>
              <button
                className="icon"
                aria-label={`Remove staged change ${i + 1}`}
                onClick={() =>
                  setStaged((s) => ({
                    ...s,
                    [active]: s[active].filter((_, j) => j !== i),
                  }))
                }
              >
                <X size={14} />
              </button>
              <pre>{JSON.stringify(change, null, 2)}</pre>
            </div>
          ))}
          <footer>
            <button onClick={() => setReviewChanges(false)}>Done</button>
          </footer>
        </Modal>
      )}
      {rowDialog && (
        <RowDialog
          info={rowDialog.inspector.info}
          old={rowDialog.old}
          onStage={(change) => stage(rowDialog.tab.id, change)}
          onClose={() => setRowDialog(null)}
        />
      )}
      {sqlImport && (
        <SqlImportDialog
          connection={sqlImport}
          timeout={preferences.timeout}
          onClose={() => setSqlImport(null)}
          onComplete={(result) => {
            setTransactionStates((s) => ({
              ...s,
              [result.connection_id]: result.transaction ?? "unknown",
            }));
            if (/connection (?:is )?closed/i.test(result.error ?? ""))
              void disconnect(sqlImport);
            else void refresh(result.connection_id);
          }}
        />
      )}
      {importDialog && (
        <ImportDialog
          connection={importDialog.connection}
          table={importDialog.inspector.table}
          info={importDialog.inspector.info}
          timeout={preferences.timeout}
          onClose={() => setImportDialog(null)}
          onComplete={(result) => {
            if (/connection (?:is )?closed/i.test(result.error ?? "")) {
              void disconnect(importDialog.connection);
            }
            setTransactionStates((s) => ({
              ...s,
              [result.connection_id]: result.transaction ?? "unknown",
            }));
            if (result.result && !result.error) {
              void executeTab(
                importDialog.tab,
                importDialog.inspector.query,
                false,
                importDialog.inspector,
              ).catch((e) => report(String(e)));
            }
          }}
        />
      )}
      {diagramConnection && (
        <Suspense
          fallback={
            <Modal
              title="Opening diagram"
              onClose={() => setDiagramConnection(null)}
            >
              <p>Loading…</p>
            </Modal>
          }
        >
          <DiagramDialog
            connection={diagramConnection}
            tables={tables[diagramConnection.id] ?? []}
            onClose={() => setDiagramConnection(null)}
          />
        </Suspense>
      )}
      {exportOpen && status && (
        <Modal title="Export results" onClose={() => setExportOpen(false)}>
          <label>
            Format
            <select
              value={exportFormat}
              onChange={(e) => setExportFormat(e.target.value)}
            >
              {["csv", "json", "jsonl", "sql", "markdown"].map((f) => (
                <option key={f} value={f}>
                  {f.toUpperCase()}
                </option>
              ))}
            </select>
          </label>
          <p className="muted">
            Exports {status.sets[set]?.rows.toLocaleString()} buffered rows from
            result {set + 1}. Files are written by Rust and replaced only after
            a complete export.
          </p>
          {exportFormat === "sql" && (
            <p className="muted">
              SQL INSERT uses the tab name as the target table. Rename/open a
              table tab before exporting.
            </p>
          )}
          <footer>
            <button
              className="primary"
              onClick={() => {
                api("export_result", {
                  id: status.id,
                  set,
                  format: exportFormat,
                  table: current?.name ?? "",
                })
                  .then((count) => {
                    if (count !== null) {
                      report(`Exported ${count.toLocaleString()} rows.`);
                      setExportOpen(false);
                    }
                  })
                  .catch((e) => report(String(e)));
              }}
            >
              <Download size={14} /> Choose destination & export
            </button>
          </footer>
        </Modal>
      )}
    </div>
  );
}
