import { useState } from "react";
import {
  cellText,
  type Cell,
  type Row,
  type TableInfo,
  type Change,
} from "../api";
import { Modal } from "./Modal";
export function RowDialog({
  info,
  old,
  onStage,
  onClose,
}: {
  info: TableInfo;
  old: Row | null;
  onStage: (change: Change) => void;
  onClose: () => void;
}) {
  const [fields, setFields] = useState(() =>
    info.columns.map((c, i) => ({
      include: !!old && !c.generated,
      kind: old?.[i].kind ?? "text",
      value: old ? cellText(old[i]) : "",
    })),
  );
  const [error, setError] = useState("");
  function submit(event: React.FormEvent) {
    event.preventDefault();
    try {
      const values: Record<string, Cell> = {};
      fields.forEach((f, i) => {
        if (!f.include || info.columns[i].generated) return;
        let cell: Cell;
        if (f.kind === "null") cell = { kind: "null" };
        else if (f.kind === "boolean")
          cell = { kind: "boolean", value: f.value === "true" };
        else if (f.kind === "json")
          cell = { kind: "json", value: JSON.parse(f.value) };
        else cell = { kind: f.kind, value: f.value };
        if (!old || JSON.stringify(cell) !== JSON.stringify(old[i]))
          values[info.columns[i].name] = cell;
      });
      if (old && !Object.keys(values).length) {
        setError("Change at least one value.");
        return;
      }
      onStage(
        old ? { kind: "update", old, values } : { kind: "insert", values },
      );
      onClose();
    } catch (e) {
      setError(`Invalid value: ${e}`);
    }
  }
  return (
    <Modal title={old ? "Edit row" : "Insert row"} onClose={onClose} wide>
      <form onSubmit={submit} className="row-form">
        <p className="muted">
          Stage values locally, then apply the batch. Existing values and the
          primary key are checked for conflicts.
        </p>
        {info.columns.map((c, i) => {
          const f = fields[i];
          const patch = (p: Partial<typeof f>) =>
            setFields((s) => s.map((v, j) => (j === i ? { ...v, ...p } : v)));
          return (
            <div className="row-field" key={c.name}>
              <label>
                {!old && !c.generated && (
                  <input
                    type="checkbox"
                    aria-label={`Set ${c.name}`}
                    checked={f.include}
                    onChange={(e) => patch({ include: e.target.checked })}
                  />
                )}
                <strong>{c.name}</strong>
                <small>
                  {c.data_type}
                  {c.primary_key ? " · primary key" : ""}
                  {c.generated ? " · generated" : ""}
                </small>
              </label>
              <select
                aria-label={`Value type for ${c.name}`}
                value={f.kind}
                disabled={c.generated || !f.include}
                onChange={(e) =>
                  patch({
                    kind: e.target.value as Cell["kind"],
                    value: e.target.value === "boolean" ? "false" : f.value,
                  })
                }
              >
                <option value="text">Text / server value</option>
                <option value="number">Number</option>
                <option value="null">NULL</option>
                <option value="binary">Binary (hex)</option>
                <option value="boolean">Boolean</option>
                <option value="json">JSON</option>
              </select>
              {f.kind === "boolean" ? (
                <select
                  aria-label={`Value for ${c.name}`}
                  disabled={c.generated || !f.include}
                  value={f.value}
                  onChange={(e) => patch({ value: e.target.value })}
                >
                  <option value="false">false</option>
                  <option value="true">true</option>
                </select>
              ) : (
                <textarea
                  aria-label={`Value for ${c.name}`}
                  rows={1}
                  disabled={c.generated || !f.include || f.kind === "null"}
                  value={f.value}
                  onChange={(e) => patch({ value: e.target.value })}
                  placeholder={
                    !f.include
                      ? "Database default"
                      : f.kind === "null"
                        ? "NULL"
                        : ""
                  }
                />
              )}
            </div>
          );
        })}
        {error && (
          <p className="error" role="alert">
            {error}
          </p>
        )}
        <footer>
          <button type="button" onClick={onClose}>
            Cancel
          </button>
          <button className="primary" type="submit">
            Stage {old ? "update" : "insert"}
          </button>
        </footer>
      </form>
    </Modal>
  );
}
