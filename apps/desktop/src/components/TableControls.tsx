import { useState } from "react";
import {
  ChevronLeft,
  ChevronRight,
  Filter,
  Plus,
  RefreshCw,
  X,
} from "lucide-react";
import type { Column, FilterOp, TableQuery } from "../api";

const operators: [FilterOp, string][] = [
  ["equal", "="],
  ["not_equal", "≠"],
  ["less", "<"],
  ["less_equal", "≤"],
  ["greater", ">"],
  ["greater_equal", "≥"],
  ["contains", "Contains"],
  ["like", "Matches (LIKE)"],
  ["is_null", "Is NULL"],
  ["is_not_null", "Is not NULL"],
];
export function TableControls({
  columns,
  query,
  active,
  busy,
  staged,
  rows,
  done,
  failed,
  onBrowse,
}: {
  columns: Column[];
  query: TableQuery;
  active: boolean;
  busy: boolean;
  staged: boolean;
  rows: number;
  done: boolean;
  failed: boolean;
  onBrowse: (query: TableQuery) => Promise<void>;
}) {
  const [filters, setFilters] = useState(query.filters);
  const [sort, setSort] = useState(query.sort[0]?.column ?? "");
  const [descending, setDescending] = useState(
    query.sort[0]?.descending ?? false,
  );
  const [limit, setLimit] = useState(query.limit);
  const disabled = busy || staged;
  const draft = {
    filters,
    sort: sort ? [{ column: sort, descending }] : [],
    limit,
    offset: 0,
  };
  return (
    <form
      className="table-controls"
      onSubmit={(e) => {
        e.preventDefault();
        void onBrowse(draft);
      }}
    >
      <div className="table-controls-bar">
        <span className="table-scope">Database</span>
        <label>
          Sort by
          <select
            aria-label="Database sort column"
            disabled={disabled}
            value={sort}
            onChange={(e) => setSort(e.target.value)}
          >
            <option value="">
              {columns.some((c) => c.primary_key)
                ? "Primary key"
                : "Unspecified"}
            </option>
            {columns.map((c) => (
              <option key={c.name} value={c.name}>
                {c.name}
              </option>
            ))}
          </select>
        </label>
        <select
          aria-label="Database sort direction"
          disabled={disabled || !sort}
          value={descending ? "desc" : "asc"}
          onChange={(e) => setDescending(e.target.value === "desc")}
        >
          <option value="asc">Ascending</option>
          <option value="desc">Descending</option>
        </select>
        <label>
          Rows
          <select
            aria-label="Rows per database page"
            disabled={disabled}
            value={limit}
            onChange={(e) => setLimit(Number(e.target.value))}
          >
            <option value={100}>100</option>
            <option value={250}>250</option>
            <option value={500}>500</option>
          </select>
        </label>
        <button type="submit" disabled={disabled}>
          <RefreshCw size={13} /> Apply to database
        </button>
        <div className="table-page-nav">
          <button
            type="button"
            aria-label="Previous database page"
            disabled={disabled || !active || query.offset === 0}
            onClick={() =>
              void onBrowse({
                ...query,
                offset: Math.max(0, query.offset - query.limit),
              })
            }
          >
            <ChevronLeft size={14} />
          </button>
          <span>DB page {Math.floor(query.offset / query.limit) + 1}</span>
          <button
            type="button"
            aria-label="Next database page"
            disabled={
              disabled ||
              !active ||
              !done ||
              failed ||
              rows < query.limit ||
              query.offset + query.limit > 1_000_000_000
            }
            onClick={() =>
              void onBrowse({ ...query, offset: query.offset + query.limit })
            }
          >
            <ChevronRight size={14} />
          </button>
        </div>
      </div>
      <details
        open={filters.length > 0 ? true : undefined}
        className="table-filters"
      >
        <summary>
          <Filter size={13} /> Column filters{" "}
          {filters.length > 0 && <small>{filters.length} · match all</small>}
        </summary>
        <fieldset disabled={disabled}>
          {filters.map((filter, i) => (
            <div className="table-filter-row" key={i}>
              <select
                aria-label={`Filter ${i + 1} column`}
                value={filter.column}
                onChange={(e) =>
                  setFilters((f) =>
                    f.map((v, j) =>
                      j === i ? { ...v, column: e.target.value } : v,
                    ),
                  )
                }
              >
                {columns.map((c) => (
                  <option key={c.name} value={c.name}>
                    {c.name}
                  </option>
                ))}
              </select>
              <select
                aria-label={`Filter ${i + 1} operator`}
                value={filter.op}
                onChange={(e) =>
                  setFilters((f) =>
                    f.map((v, j) =>
                      j === i ? { ...v, op: e.target.value as FilterOp } : v,
                    ),
                  )
                }
              >
                {operators.map(([value, label]) => (
                  <option key={value} value={value}>
                    {label}
                  </option>
                ))}
              </select>
              <input
                aria-label={`Filter ${i + 1} value`}
                placeholder={
                  filter.op === "like" ? "SQL LIKE pattern…" : "Value…"
                }
                value={filter.value}
                disabled={
                  filter.op === "is_null" || filter.op === "is_not_null"
                }
                onChange={(e) =>
                  setFilters((f) =>
                    f.map((v, j) =>
                      j === i ? { ...v, value: e.target.value } : v,
                    ),
                  )
                }
              />
              <button
                type="button"
                className="icon"
                aria-label={`Remove filter ${i + 1}`}
                onClick={() => setFilters((f) => f.filter((_, j) => i !== j))}
              >
                <X size={13} />
              </button>
            </div>
          ))}
          <button
            type="button"
            disabled={filters.length >= 20 || !columns.length}
            onClick={() =>
              setFilters((f) => [
                ...f,
                { column: columns[0].name, op: "equal", value: "" },
              ])
            }
          >
            <Plus size={13} /> Add filter
          </button>
          {!!filters.length && (
            <button
              type="button"
              onClick={() => {
                setFilters([]);
                void onBrowse({ ...draft, filters: [] });
              }}
            >
              Clear filters
            </button>
          )}
          <small className="muted">
            Applies across the table. Contains treats % and _ literally; LIKE
            uses database pattern rules.
          </small>
        </fieldset>
      </details>
      {staged && (
        <small className="muted">
          Apply or discard staged edits before changing table controls.
        </small>
      )}
      {!active && (
        <small className="muted">
          Showing custom SQL. Apply these controls to return to table browsing.
        </small>
      )}
      {!columns.some((c) => c.primary_key) && (
        <small className="muted">
          No primary key: sort ties can move between pages. Concurrent writes
          can also shift page boundaries.
        </small>
      )}
    </form>
  );
}
