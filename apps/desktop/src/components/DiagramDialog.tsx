import { useEffect, useRef, useState } from "react";
import {
  Download,
  Maximize,
  Minus,
  Plus,
  Save,
  LayoutGrid,
  RefreshCw,
} from "lucide-react";
import { api, type Connection, type Diagram, type Table } from "../api";
import {
  autoLayout,
  bounded,
  CARD_WIDTH,
  cardHeight,
  HEADER,
  relationPath,
  restoreDiagram,
  ROW,
  short,
  tableKey,
  type DiagramLayout,
  type Point,
} from "../diagram";
import { Modal } from "./Modal";

export function DiagramDialog({
  connection,
  tables,
  onClose,
}: {
  connection: Connection;
  tables: Table[];
  onClose: () => void;
}) {
  const [model, setModel] = useState<Diagram | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [positions, setPositions] = useState<Record<string, Point>>({});
  const [zoom, setZoom] = useState(1),
    [pan, setPan] = useState<Point>({ x: 20, y: 20 });
  const [search, setSearch] = useState(""),
    [busy, setBusy] = useState(true),
    [ready, setReady] = useState(false);
  const [error, setError] = useState(""),
    [message, setMessage] = useState("");
  const [focused, setFocused] = useState("");
  const canvas = useRef<HTMLDivElement>(null),
    live = useRef(true);
  const drag = useRef<{
    key: string | null;
    start: Point;
    original: Point;
  } | null>(null);
  const layout = useRef<DiagramLayout | null>(null);
  layout.current = ready
    ? {
        tables: selected
          .map((k) => tables.find((t) => tableKey(t) === k))
          .filter((t): t is Table => !!t),
        positions,
        zoom,
        pan,
      }
    : null;
  const documentId = `diagram-${connection.id}`;
  useEffect(() => {
    live.current = true;
    let mounted = true;
    api("load_document", { id: documentId })
      .then(async (data) => {
        if (!mounted) return;
        setReady(true);
        const saved = restoreDiagram(data);
        if (saved) {
          setSelected(
            saved.tables
              .map(tableKey)
              .filter((k) => tables.some((t) => tableKey(t) === k)),
          );
          setPositions(saved.positions);
          setZoom(saved.zoom);
          setPan(saved.pan);
          const selectedTables = tables.filter((t) =>
            saved.tables.some((st) => tableKey(st) === tableKey(t)),
          );
          if (selectedTables.length) {
            const next = await api("diagram_tables", {
              id: connection.id,
              tables: selectedTables,
            });
            if (!mounted) return;
            const defaults = autoLayout(next);
            setPositions(
              Object.fromEntries(
                next.tables.map((t) => [
                  tableKey(t.table),
                  saved.positions[tableKey(t.table)] ??
                    defaults[tableKey(t.table)],
                ]),
              ),
            );
            setModel(next);
          }
        }
      })
      .catch((e) => {
        if (mounted) setError(String(e));
      })
      .finally(() => {
        if (mounted) setBusy(false);
      });
    return () => {
      mounted = false;
      live.current = false;
    };
  }, [documentId, tables, connection.id]);
  async function save(close = false) {
    if (busy) return;
    if (!layout.current) {
      if (close) onClose();
      return;
    }
    setBusy(true);
    setError("");
    try {
      await api("save_document", { id: documentId, data: layout.current });
      setMessage("Layout saved locally");
      if (close) onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      if (live.current) setBusy(false);
    }
  }
  async function load() {
    if (!selected.length) return;
    setBusy(true);
    setError("");
    setMessage("");
    try {
      const next = await api("diagram_tables", {
        id: connection.id,
        tables: tables.filter((t) => selected.includes(tableKey(t))),
      });
      if (!live.current) return;
      setModel(next);
      const defaults = autoLayout(next);
      setPositions((old) =>
        Object.fromEntries(
          next.tables.map((t) => [
            tableKey(t.table),
            old[tableKey(t.table)] ?? defaults[tableKey(t.table)],
          ]),
        ),
      );
    } catch (e) {
      if (live.current) setError(String(e));
    } finally {
      if (live.current) setBusy(false);
    }
  }
  function fit() {
    if (!model?.tables.length || !canvas.current) return;
    const nodes = model.tables
      .map((t) => ({ t, p: positions[tableKey(t.table)] }))
      .filter((n) => n.p);
    const left = Math.min(...nodes.map((n) => n.p.x)),
      top = Math.min(...nodes.map((n) => n.p.y));
    const width = Math.max(...nodes.map((n) => n.p.x + CARD_WIDTH)) - left + 80,
      height =
        Math.max(...nodes.map((n) => n.p.y + cardHeight(n.t))) - top + 80;
    const scale = Math.max(
      0.1,
      Math.min(
        1,
        canvas.current.clientWidth / width,
        canvas.current.clientHeight / height,
      ),
    );
    setZoom(scale);
    setPan({ x: 40 - left * scale, y: 40 - top * scale });
  }
  function changeZoom(next: number) {
    next = Math.max(0.1, Math.min(3, next));
    const cx = (canvas.current?.clientWidth ?? 600) / 2,
      cy = (canvas.current?.clientHeight ?? 400) / 2;
    setPan((p) => ({
      x: bounded(cx - ((cx - p.x) * next) / zoom),
      y: bounded(cy - ((cy - p.y) * next) / zoom),
    }));
    setZoom(next);
  }
  async function exportSvg() {
    if (!model) return;
    setBusy(true);
    setError("");
    try {
      const bytes = await api("export_diagram", { model, positions });
      if (bytes !== null)
        setMessage(`SVG exported · ${bytes.toLocaleString()} bytes`);
    } catch (e) {
      setError(String(e));
    } finally {
      if (live.current) setBusy(false);
    }
  }
  const relationships =
    model?.tables.flatMap((source) =>
      source.relationships.map((fk) => ({
        source,
        fk,
        target: model.tables.find(
          (t) =>
            t.table.schema === fk.target_schema &&
            t.table.name === fk.target_table,
        ),
      })),
    ) ?? [];
  const visible = tables
    .filter((t) =>
      `${t.schema}.${t.name}`.toLowerCase().includes(search.toLowerCase()),
    )
    .slice(0, 200);
  const displayed = model?.tables.length ?? 0;
  const selectionChanged =
    model &&
    (selected.length !== displayed ||
      model.tables.some((t) => !selected.includes(tableKey(t.table))));
  return (
    <Modal
      title={`Relationships · ${connection.name}`}
      className="diagram-modal"
      onClose={() => void save(true)}
    >
      <div className="diagram-toolbar">
        <button disabled={busy || !selected.length} onClick={() => void load()}>
          <RefreshCw size={14} /> Load selected
        </button>
        <button
          disabled={busy || !model}
          onClick={() => {
            if (model) {
              setPositions(autoLayout(model));
              setZoom(1);
              setPan({ x: 20, y: 20 });
            }
          }}
        >
          <LayoutGrid size={14} /> Auto-layout
        </button>
        <button disabled={busy || !model} onClick={fit}>
          <Maximize size={14} /> Fit
        </button>
        <button
          aria-label="Zoom out diagram"
          disabled={busy || !model}
          onClick={() => changeZoom(zoom / 1.2)}
        >
          <Minus size={14} />
        </button>
        <span>{Math.round(zoom * 100)}%</span>
        <button
          aria-label="Zoom in diagram"
          disabled={busy || !model}
          onClick={() => changeZoom(zoom * 1.2)}
        >
          <Plus size={14} />
        </button>
        <button disabled={busy || !ready} onClick={() => void save()}>
          <Save size={14} /> Save layout
        </button>
        <button disabled={busy || !model} onClick={() => void exportSvg()}>
          <Download size={14} /> Export SVG
        </button>
      </div>
      <div className="diagram-workspace">
        <aside className="diagram-picker">
          <input
            aria-label="Find diagram tables"
            placeholder="Find a table…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
          />
          <small>{selected.length} selected · up to 50</small>
          <button
            disabled={busy || !selected.length}
            onClick={() => setSelected([])}
          >
            Clear selection
          </button>
          <div className="diagram-table-list">
            {visible.map((t) => {
              const key = tableKey(t),
                checked = selected.includes(key);
              return (
                <label key={key}>
                  <input
                    type="checkbox"
                    checked={checked}
                    disabled={busy || (!checked && selected.length >= 50)}
                    onChange={() =>
                      setSelected((s) =>
                        checked ? s.filter((k) => k !== key) : [...s, key],
                      )
                    }
                  />
                  <span title={`${t.schema}.${t.name}`}>
                    {t.name}
                    <small>{t.schema}</small>
                  </span>
                </label>
              );
            })}
          </div>
          {tables.length > visible.length && (
            <small>
              Showing up to 200 matches. Search to narrow the schema.
            </small>
          )}
          <small>Metadata only. No table rows are read.</small>
        </aside>
        <div
          className="diagram-canvas"
          ref={canvas}
          onPointerDown={(e) => {
            if (e.button !== 0 || busy) return;
            e.currentTarget.setPointerCapture(e.pointerId);
            drag.current = {
              key: null,
              start: { x: e.clientX, y: e.clientY },
              original: pan,
            };
          }}
          onPointerMove={(e) => {
            const d = drag.current;
            if (!d) return;
            const dx = e.clientX - d.start.x,
              dy = e.clientY - d.start.y;
            if (d.key)
              setPositions((p) => ({
                ...p,
                [d.key!]: {
                  x: bounded(d.original.x + dx / zoom),
                  y: bounded(d.original.y + dy / zoom),
                },
              }));
            else
              setPan({
                x: bounded(d.original.x + dx),
                y: bounded(d.original.y + dy),
              });
          }}
          onPointerUp={() => {
            drag.current = null;
          }}
          onPointerCancel={() => {
            drag.current = null;
          }}
        >
          {model ? (
            <svg
              role="group"
              aria-label="Relationship diagram"
              width="100%"
              height="100%"
            >
              <defs>
                <marker
                  id="diagram-arrow"
                  markerWidth="8"
                  markerHeight="8"
                  refX="7"
                  refY="4"
                  orient="auto"
                >
                  <path d="M0 0 L8 4 L0 8" fill="var(--accent)" />
                </marker>
              </defs>
              <g transform={`translate(${pan.x} ${pan.y}) scale(${zoom})`}>
                {relationships.flatMap(({ source, fk, target }, i) =>
                  target &&
                  positions[tableKey(source.table)] &&
                  positions[tableKey(target.table)]
                    ? fk.columns.map((from, j) => (
                        <path
                          key={`${i}-${j}`}
                          d={relationPath(
                            source,
                            target,
                            from,
                            fk.target_columns[j],
                            positions[tableKey(source.table)],
                            positions[tableKey(target.table)],
                          )}
                          fill="none"
                          stroke="var(--accent)"
                          strokeWidth={1.5}
                          markerEnd="url(#diagram-arrow)"
                        >
                          <title>
                            {short(fk.name, 120)}:{" "}
                            {short(source.table.name, 80)}.{short(from, 80)} →{" "}
                            {target.table.name}.
                            {fk.target_columns[j] ?? "implicit key"}
                          </title>
                        </path>
                      ))
                    : [],
                )}
                {model.tables.map((t) => {
                  const key = tableKey(t.table),
                    p = positions[key];
                  if (!p) return null;
                  return (
                    <g
                      key={key}
                      data-table={key}
                      transform={`translate(${p.x} ${p.y})`}
                      role="button"
                      tabIndex={0}
                      aria-label={`Table ${t.table.schema}.${t.table.name}. Arrow keys move this table.`}
                      onPointerDown={(e) => {
                        e.stopPropagation();
                        if (e.button !== 0 || busy) return;
                        e.currentTarget.setPointerCapture(e.pointerId);
                        setFocused(key);
                        drag.current = {
                          key,
                          start: { x: e.clientX, y: e.clientY },
                          original: p,
                        };
                      }}
                      onFocus={() => setFocused(key)}
                      onKeyDown={(e) => {
                        if (busy) return;
                        const delta = (
                          {
                            ArrowLeft: [-20, 0],
                            ArrowRight: [20, 0],
                            ArrowUp: [0, -20],
                            ArrowDown: [0, 20],
                          } as Record<string, number[]>
                        )[e.key];
                        if (delta) {
                          e.preventDefault();
                          setPositions((old) => ({
                            ...old,
                            [key]: {
                              x: bounded(p.x + delta[0]),
                              y: bounded(p.y + delta[1]),
                            },
                          }));
                        }
                      }}
                    >
                      <rect
                        width={CARD_WIDTH}
                        height={cardHeight(t)}
                        rx={8}
                        fill="var(--surface)"
                        stroke={
                          focused === key ? "var(--accent)" : "var(--border)"
                        }
                        strokeWidth={focused === key ? 2 : 1}
                      />
                      <text x={12} y={17} fill="var(--muted)" fontSize={10}>
                        {short(t.table.schema, 32)}
                      </text>
                      <text
                        x={12}
                        y={36}
                        fill="var(--text)"
                        fontSize={13}
                        fontWeight={600}
                      >
                        {short(t.table.name, 30)}
                        <title>{t.table.name}</title>
                      </text>
                      <line
                        x1={0}
                        x2={CARD_WIDTH}
                        y1={HEADER - 4}
                        y2={HEADER - 4}
                        stroke="var(--border)"
                      />
                      {t.columns.map((c, i) => (
                        <g key={c.name}>
                          <text
                            x={12}
                            y={HEADER + ROW * i + 17}
                            fill={
                              c.primary_key ? "var(--accent)" : "var(--text)"
                            }
                            fontSize={11}
                          >
                            {c.primary_key
                              ? "◆ "
                              : t.relationships.some((r) =>
                                    r.columns.includes(c.name),
                                  )
                                ? "↗ "
                                : "  "}
                            {short(c.name, 20)}
                            <title>
                              {c.name}
                              {c.primary_key ? " · primary key" : ""}
                              {c.nullable ? " · nullable" : " · required"}
                            </title>
                          </text>
                          <text
                            x={185}
                            y={HEADER + ROW * i + 17}
                            fill="var(--muted)"
                            fontSize={10}
                          >
                            {short(c.data_type, 12)}
                            <title>{c.data_type}</title>
                          </text>
                        </g>
                      ))}
                    </g>
                  );
                })}
              </g>
            </svg>
          ) : (
            <div className="diagram-empty">
              <LayoutGrid size={32} />
              <h3>Your schema, connected</h3>
              <p>Select tables, then load their relationships.</p>
            </div>
          )}
        </div>
      </div>
      <div className="diagram-status" role="status">
        {selectionChanged && (
          <span>Selection changed · choose Load selected. </span>
        )}
        {busy ? (
          "Working…"
        ) : error ? (
          <span className="error">{error}</span>
        ) : (
          message ||
          `${displayed} tables · ${relationships.length} foreign keys · ◆ primary key · ↗ foreign key · drag background to pan`
        )}
      </div>
      {model && (
        <details className="diagram-relations">
          <summary>
            Relationships · {relationships.filter((r) => !r.target).length}{" "}
            outside this selection
          </summary>
          {relationships.map(({ source, fk, target }, i) => (
            <p key={i}>
              {source.table.schema}.{source.table.name} ({fk.columns.join(", ")}
              ) → {fk.target_schema}.{fk.target_table} (
              {fk.target_columns.map((c) => c ?? "implicit key").join(", ")}) ·{" "}
              {fk.name}
              {!target &&
                " · select the target table to draw this relationship"}
            </p>
          ))}
        </details>
      )}
    </Modal>
  );
}
