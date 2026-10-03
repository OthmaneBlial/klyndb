import { SQLDialect, StandardSQL } from "@codemirror/lang-sql";
export const ClickHouseSQL = SQLDialect.define({
  ...StandardSQL.spec,
  backslashEscapes: true,
  identifierQuotes: '`"',
});
import { EditorState, type TransactionSpec } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
export function currentStatementRange(
  state: EditorState,
): { from: number; to: number } | null {
  const selected = state.selection.main;
  if (!selected.empty) return { from: selected.from, to: selected.to };
  // Never widen an unresolved statement into an entire SQL batch.
  const tree = ensureSyntaxTree(state, state.doc.length, 100);
  if (!tree) return null;
  let node = tree.resolveInner(selected.head, 1);
  while (node.parent && node.name !== "Statement") node = node.parent;
  if (node.name === "Statement") return { from: node.from, to: node.to };
  let previous = tree.topNode.childBefore(selected.head);
  while (previous && previous.name !== "Statement")
    previous = previous.prevSibling;
  return previous ? { from: previous.from, to: previous.to } : null;
}
export function currentStatement(state: EditorState): string {
  const range = currentStatementRange(state);
  return range ? state.sliceDoc(range.from, range.to) : "";
}
export interface SqlSubmission {
  document: string;
  sql: string;
  from: number;
}
export function captureSubmission(
  state: EditorState,
  sql: string,
): SqlSubmission | null {
  if (!sql) return null;
  const document = state.doc.toString();
  if (sql === document) return { document, sql, from: 0 };
  const range = currentStatementRange(state);
  return range && state.sliceDoc(range.from, range.to) === sql
    ? { document, sql, from: range.from }
    : null;
}
export function sqlErrorPosition(
  state: EditorState,
  source: SqlSubmission,
  offset: number,
): number | null {
  if (
    state.doc.toString() !== source.document ||
    !Number.isSafeInteger(offset) ||
    offset < 0 ||
    offset > source.sql.length
  )
    return null;
  const position = source.from + offset;
  if (
    position < 0 ||
    position > state.doc.length ||
    state.sliceDoc(source.from, source.from + source.sql.length) !== source.sql
  )
    return null;
  const before = (
      position > 0 ? state.sliceDoc(position - 1, position) : ""
    ).charCodeAt(0),
    after = (
      position < state.doc.length ? state.sliceDoc(position, position + 1) : ""
    ).charCodeAt(0);
  if (
    before >= 0xd800 &&
    before <= 0xdbff &&
    after >= 0xdc00 &&
    after <= 0xdfff
  )
    return null;
  return position;
}

export function replaceDocument(
  editor: { state: EditorState; dispatch: (spec: TransactionSpec) => void },
  text: string,
) {
  if (editor.state.doc.toString() !== text)
    editor.dispatch({
      changes: { from: 0, to: editor.state.doc.length, insert: text },
    });
}
