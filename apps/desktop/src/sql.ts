import { EditorState } from "@codemirror/state";
import { syntaxTree } from "@codemirror/language";
export function currentStatement(state: EditorState): string {
  const selected = state.selection.main;
  if (!selected.empty) return state.sliceDoc(selected.from, selected.to);
  let node = syntaxTree(state).resolveInner(selected.head, 1);
  while (node.parent && node.name !== "Statement") node = node.parent;
  return node.name === "Statement"
    ? state.sliceDoc(node.from, node.to)
    : state.doc.toString();
}
