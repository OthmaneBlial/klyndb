import { SQLDialect, StandardSQL } from "@codemirror/lang-sql";
export const ClickHouseSQL = SQLDialect.define({ ...StandardSQL.spec, backslashEscapes: true, identifierQuotes: "`\"" });
import { EditorState, type TransactionSpec } from "@codemirror/state";
import { ensureSyntaxTree } from "@codemirror/language";
export function currentStatement(state: EditorState): string {
  const selected = state.selection.main;
  if (!selected.empty) return state.sliceDoc(selected.from, selected.to);
  // Never widen an unresolved statement into an entire SQL batch.
  const tree = ensureSyntaxTree(state, state.doc.length, 100);
  if (!tree) return "";
  let node = tree.resolveInner(selected.head, 1);
  while (node.parent && node.name !== "Statement") node = node.parent;
  if (node.name === "Statement") return state.sliceDoc(node.from, node.to);
  let previous = tree.topNode.childBefore(selected.head);
  while (previous && previous.name !== "Statement")
    previous = previous.prevSibling;
  return previous ? state.sliceDoc(previous.from, previous.to) : "";
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
