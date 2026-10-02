import { useEffect, useRef } from "react";
import { EditorState } from "@codemirror/state";
import {
  EditorView,
  keymap,
  lineNumbers,
  highlightActiveLine,
  drawSelection,
  highlightActiveLineGutter,
} from "@codemirror/view";
import { defaultKeymap, history, historyKeymap } from "@codemirror/commands";
import {
  defaultHighlightStyle,
  syntaxHighlighting,
  bracketMatching,
} from "@codemirror/language";
import {
  autocompletion,
  closeBrackets,
  closeBracketsKeymap,
} from "@codemirror/autocomplete";
import { searchKeymap, highlightSelectionMatches } from "@codemirror/search";
import { sql, SQLite, PostgreSQL, MySQL } from "@codemirror/lang-sql";
import { currentStatement } from "../sql";
export interface EditorHandle {
  runText: () => string;
  allText: () => string;
  replace: (sql: string) => void;
}
export function SqlEditor({
  value,
  engine,
  schema,
  onChange,
  onRun,
  editorRef,
}: {
  value: string;
  engine: string;
  schema: Record<string, string[]>;
  onChange: (s: string) => void;
  onRun: (s: string) => void;
  editorRef: React.RefObject<EditorHandle | null>;
}) {
  const element = useRef<HTMLDivElement>(null),
    view = useRef<EditorView | null>(null);
  const callbacks = useRef({ onChange, onRun });
  callbacks.current = { onChange, onRun };
  useEffect(() => {
    const state = EditorState.create({
      doc: value,
      extensions: [
        lineNumbers(),
        highlightActiveLine(),
        highlightActiveLineGutter(),
        drawSelection(),
        history(),
        bracketMatching(),
        closeBrackets(),
        highlightSelectionMatches(),
        syntaxHighlighting(defaultHighlightStyle),
        autocompletion(),
        sql({
          dialect:
            engine === "sqlite"
              ? SQLite
              : engine === "mysql"
                ? MySQL
                : PostgreSQL,
          schema,
        }),
        keymap.of([
          {
            key: "Mod-Enter",
            run: (view) => {
              callbacks.current.onRun(currentStatement(view.state));
              return true;
            },
          },
          ...defaultKeymap,
          ...historyKeymap,
          ...closeBracketsKeymap,
          ...searchKeymap,
        ]),
        EditorView.updateListener.of((update) => {
          if (update.docChanged)
            callbacks.current.onChange(update.state.doc.toString());
        }),
        EditorView.theme({
          "&": { height: "100%", fontSize: "13px" },
          ".cm-content": {
            fontFamily: "'SFMono-Regular', Consolas, monospace",
            padding: "16px 0",
          },
          ".cm-gutters": {
            background: "transparent",
            border: "none",
            color: "var(--muted)",
          },
          ".cm-lineNumbers .cm-gutterElement": { padding: "0 16px 0 12px" },
          ".cm-activeLine": { background: "var(--selection)" },
          ".cm-activeLineGutter": { background: "transparent" },
          ".cm-cursor": { borderLeftColor: "var(--accent)" },
          ".cm-scroller": { overflow: "auto" },
          ".cm-tooltip": {
            background: "var(--surface)",
            border: "1px solid var(--border)",
            color: "var(--text)",
          },
          ".cm-selectionBackground": {
            background: "var(--selection) !important",
          },
        }),
      ],
    });
    const editor = new EditorView({ state, parent: element.current! });
    view.current = editor;
    editorRef.current = {
      runText: () => currentStatement(editor.state),
      allText: () => editor.state.doc.toString(),
      replace: (text) =>
        editor.dispatch({
          changes: { from: 0, to: editor.state.doc.length, insert: text },
        }),
    };
    return () => {
      editor.destroy();
      view.current = null;
      editorRef.current = null;
    };
    // Each tab owns its editor; callbacks retain the current handlers.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [engine, schema]);
  return <div className="sql-editor" ref={element} />;
}
