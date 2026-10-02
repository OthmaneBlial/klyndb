import { useEffect, useRef, useState, useMemo } from "react";
import {
  ChevronLeft,
  ChevronRight,
  Copy,
  ArrowDown,
  ArrowUp,
  Search,
  Pencil,
  Trash2,
} from "lucide-react";
import { api, cellText, type Cell, type ResultSet, type Row } from "../api";
import { Modal } from "./Modal";
const PAGE = 500,
  ROW = 30;
export function ResultGrid({
  id,
  set,
  metadata,
  onError,
  onEdit,
  onDelete,
  rowOffset,
}: {
  id: string;
  set: number;
  metadata: ResultSet;
  onError: (s: string) => void;
  onEdit?: (row: Row) => void;
  onDelete?: (row: Row) => void;
  rowOffset?: number;
}) {
  const [page, setPage] = useState(0),
    [rows, setRows] = useState<Row[]>([]),
    [scroll, setScroll] = useState(0),
    [height, setHeight] = useState(320),
    [widths, setWidths] = useState<Record<number, number>>({}),
    [order, setOrder] = useState<number[]>(metadata.columns.map((_, i) => i)),
    [sort, setSort] = useState<{ column: number; desc: boolean } | null>(null),
    [filter, setFilter] = useState(""),
    [viewer, setViewer] = useState<Cell | null>(null),
    [drag, setDrag] = useState<number | null>(null);
  const viewport = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let live = true;
    setRows([]);
    api("result_page", { id, set, offset: page * PAGE, limit: PAGE })
      .then((r) => {
        if (live) setRows(r);
      })
      .catch((e) => onError(String(e)));
    return () => {
      live = false;
    };
  }, [id, set, page, metadata.rows, onError]);
  useEffect(() => {
    const observer = new ResizeObserver((entries) =>
      setHeight(entries[0].contentRect.height),
    );
    if (viewport.current) observer.observe(viewport.current);
    return () => observer.disconnect();
  }, []);
  const display = useMemo(() => {
    let result = rows.map((row, index) => ({ row, index }));
    if (filter)
      result = result.filter(({ row }) =>
        row.some((c) =>
          cellText(c).toLowerCase().includes(filter.toLowerCase()),
        ),
      );
    if (sort)
      result.sort(
        (a, b) =>
          cellText(a.row[sort.column]).localeCompare(
            cellText(b.row[sort.column]),
            undefined,
            { numeric: true },
          ) * (sort.desc ? -1 : 1),
      );
    return result;
  }, [rows, filter, sort]);
  const start = Math.max(0, Math.floor(scroll / ROW) - 4),
    end = Math.min(display.length, start + Math.ceil(height / ROW) + 8);
  const template = `54px ${onEdit || onDelete ? "78px " : ""}${order.map((i) => `${widths[i] ?? 180}px`).join(" ")}`;
  function resize(event: React.PointerEvent, index: number) {
    event.preventDefault();
    event.stopPropagation();
    const x = event.clientX,
      original = widths[index] ?? 180;
    event.currentTarget.setPointerCapture(event.pointerId);
    const move = (e: PointerEvent) =>
      setWidths((w) => ({
        ...w,
        [index]: Math.max(60, original + e.clientX - x),
      }));
    const stop = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", stop);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", stop, { once: true });
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
    } catch (e) {
      onError(`Could not copy: ${e}`);
    }
  }
  return (
    <div className="grid-panel">
      <div className="grid-controls">
        <div className="search">
          <Search size={14} />
          <input
            aria-label="Filter current result page"
            placeholder="Filter this page…"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
          />
        </div>
        <span className="muted">Sort & filter apply to this page</span>
        <button
          title="Copy page as CSV"
          onClick={() =>
            void copy(
              [metadata.columns, ...display.map(({ row }) => row.map(cellText))]
                .map((row) =>
                  row.map((c) => `"${c.replaceAll('"', '""')}"`).join(","),
                )
                .join("\n"),
            )
          }
        >
          <Copy size={14} /> Copy page
        </button>
      </div>
      <div
        className="grid-viewport"
        ref={viewport}
        onScroll={(e) => setScroll(e.currentTarget.scrollTop)}
        role="table"
        aria-label="Query results"
        aria-rowcount={rowOffset === undefined ? metadata.rows + 1 : -1}
      >
        <div
          className="grid-header"
          role="row"
          style={{ gridTemplateColumns: template }}
        >
          <div className="row-number" role="columnheader">
            #
          </div>
          {(onEdit || onDelete) && <div role="columnheader">Edit</div>}
          {order.map((i) => (
            <div
              role="columnheader"
              key={i}
              draggable
              onDragStart={() => setDrag(i)}
              onDragOver={(e) => e.preventDefault()}
              onDrop={() => {
                if (drag !== null) {
                  setOrder((o) => {
                    const next = o.filter((c) => c !== drag);
                    next.splice(next.indexOf(i), 0, drag);
                    return next;
                  });
                  setDrag(null);
                }
              }}
            >
              <button
                onClick={() =>
                  setSort((s) => ({
                    column: i,
                    desc: s?.column === i ? !s.desc : false,
                  }))
                }
              >
                {metadata.columns[i]}
                {sort?.column === i &&
                  (sort.desc ? <ArrowDown size={12} /> : <ArrowUp size={12} />)}
              </button>
              <button
                className="copy-column"
                title={`Copy column ${metadata.columns[i]}`}
                onClick={() =>
                  void copy(
                    display.map(({ row }) => cellText(row[i])).join("\n"),
                  )
                }
              >
                <Copy size={11} />
              </button>
              <span
                className="column-resizer"
                onPointerDown={(e) => resize(e, i)}
              />
            </div>
          ))}
        </div>
        <div
          style={{
            height: display.length * ROW,
            position: "relative",
            minWidth: "max-content",
          }}
        >
          {display.slice(start, end).map(({ row, index }, position) => (
            <div
              className="grid-row"
              role="row"
              aria-rowindex={(rowOffset ?? 0) + page * PAGE + index + 2}
              key={index}
              style={{
                top: (start + position) * ROW,
                gridTemplateColumns: template,
              }}
            >
              <button
                className="row-number"
                title="Copy row as JSON"
                onClick={() =>
                  void copy(
                    JSON.stringify({ columns: metadata.columns, values: row }),
                  )
                }
              >
                {(rowOffset ?? 0) + page * PAGE + index + 1}
              </button>
              {(onEdit || onDelete) && (
                <div className="row-actions" role="cell">
                  {onEdit && (
                    <button
                      className="icon"
                      aria-label={`Edit row ${(rowOffset ?? 0) + page * PAGE + index + 1}`}
                      onClick={() => onEdit(row)}
                    >
                      <Pencil size={13} />
                    </button>
                  )}
                  {onDelete && (
                    <button
                      className="icon"
                      aria-label={`Stage deletion of row ${(rowOffset ?? 0) + page * PAGE + index + 1}`}
                      onClick={() => onDelete(row)}
                    >
                      <Trash2 size={13} />
                    </button>
                  )}
                </div>
              )}
              {order.map((i) => (
                <button
                  role="cell"
                  className={`grid-cell ${row[i].kind}`}
                  key={i}
                  title="Click to copy · double-click to inspect"
                  onClick={() => void copy(cellText(row[i]))}
                  onDoubleClick={() => setViewer(row[i])}
                >
                  {row[i].kind === "null" ? (
                    <span>NULL</span>
                  ) : row[i].kind === "binary" ? (
                    `0x${cellText(row[i]).slice(0, 32)}`
                  ) : (
                    cellText(row[i])
                  )}
                </button>
              ))}
            </div>
          ))}
        </div>
        {display.length === 0 && (
          <div className="grid-empty">
            {filter ? "No matching rows on this page." : "No rows returned."}
          </div>
        )}
      </div>
      <div className="pagination">
        <span>
          {rowOffset !== undefined && metadata.rows > 0
            ? `Rows ${(rowOffset + 1).toLocaleString()}–${(rowOffset + metadata.rows).toLocaleString()} · `
            : ""}
          {metadata.rows.toLocaleString()} rows
          {metadata.truncated && " · row limit reached"}
        </span>
        {rowOffset === undefined && (
          <div>
            <button
              aria-label="Previous result page"
              disabled={page === 0}
              onClick={() => {
                setPage((p) => p - 1);
                viewport.current?.scrollTo(0, 0);
              }}
            >
              <ChevronLeft size={15} />
            </button>
            <label>
              Page{" "}
              <input
                aria-label="Result page number"
                type="number"
                min={1}
                max={Math.max(1, Math.ceil(metadata.rows / PAGE))}
                value={page + 1}
                onChange={(e) => {
                  setPage(
                    Math.max(
                      0,
                      Math.min(
                        Math.ceil(metadata.rows / PAGE) - 1,
                        Number(e.target.value) - 1,
                      ),
                    ),
                  );
                  viewport.current?.scrollTo(0, 0);
                }}
              />{" "}
              of {Math.max(1, Math.ceil(metadata.rows / PAGE)).toLocaleString()}
            </label>
            <button
              aria-label="Next result page"
              disabled={(page + 1) * PAGE >= metadata.rows}
              onClick={() => {
                setPage((p) => p + 1);
                viewport.current?.scrollTo(0, 0);
              }}
            >
              <ChevronRight size={15} />
            </button>
          </div>
        )}
      </div>
      {viewer && (
        <Modal
          title={`Cell · ${viewer.kind}`}
          onClose={() => setViewer(null)}
          wide
        >
          <pre className="cell-viewer">
            {viewer.kind === "json"
              ? JSON.stringify(viewer.value, null, 2)
              : (() => {
                  try {
                    return JSON.stringify(
                      JSON.parse(cellText(viewer)),
                      null,
                      2,
                    );
                  } catch {
                    return viewer.kind === "null" ? "NULL" : cellText(viewer);
                  }
                })()}
          </pre>
          <footer>
            <button onClick={() => void copy(cellText(viewer))}>
              <Copy size={14} /> Copy value
            </button>
          </footer>
        </Modal>
      )}
    </div>
  );
}
