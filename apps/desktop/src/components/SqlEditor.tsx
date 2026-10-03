import { useEffect, useMemo, useRef } from "react";
import { Compartment, EditorState } from "@codemirror/state";
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
import { sql, SQLite, PostgreSQL, MySQL, MSSQL } from "@codemirror/lang-sql";
import {
  ClickHouseSQL,
  currentStatement,
  replaceDocument,
  captureSubmission,
  sqlErrorPosition,
  type SqlSubmission,
} from "../sql";
import { tableCompletion, type CompletionSchema } from "../completion";
import type { Table } from "../api";
export interface EditorHandle {
  runText: () => string;
  allText: () => string;
  replace: (sql: string) => void;
  sourceFor: (sql: string) => SqlSubmission | null;
  locateError: (source: SqlSubmission, offset: number) => boolean;
}
export function SqlEditor({
  value,
  engine,
  schema,
  loadColumns,
  onError,
  onChange,
  onRun,
  editorRef,
}: {
  value: string;
  engine: string;
  schema: CompletionSchema;
  loadColumns: (table: Table) => Promise<string[]>;
  onError: (message: string) => void;
  onChange: (s: string) => void;
  onRun: (s: string) => void;
  editorRef: React.RefObject<EditorHandle | null>;
}) {
  const element = useRef<HTMLDivElement>(null),
    view = useRef<EditorView | null>(null);
  const callbacks = useRef({ onChange, onRun, onError });
  callbacks.current = { onChange, onRun, onError };
  const language = useRef(new Compartment());
  const dialect =
    engine === "sqlite"
      ? SQLite
      : engine === "mysql"
        ? MySQL
        : engine === "mssql"
          ? MSSQL
          : engine === "clickhouse"
            ? ClickHouseSQL
            : PostgreSQL;
  const completion = useMemo(() => {
    const source = tableCompletion(schema, dialect, loadColumns);
    return async (...args: Parameters<typeof source>) => {
      try {
        return await source(...args);
      } catch (error) {
        if (!args[0].aborted)
          callbacks.current.onError(
            `Column completion failed: ${String(error)}`,
          );
        return null;
      }
    };
  }, [schema, dialect, loadColumns]);
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
        language.current.of([
          sql({ dialect }),
          dialect.language.data.of({ autocomplete: completion }),
        ]),
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
      replace: (text) => replaceDocument(editor, text),
      sourceFor: (text) => captureSubmission(editor.state, text),
      locateError: (source, offset) => {
        const position = sqlErrorPosition(editor.state, source, offset);
        if (position === null) return false;
        editor.dispatch({
          selection: { anchor: position },
          effects: EditorView.scrollIntoView(position, { y: "center" }),
        });
        editor.focus();
        return true;
      },
    };
    return () => {
      editor.destroy();
      view.current = null;
      editorRef.current = null;
    };
    // Each tab owns its editor; callbacks retain the current handlers.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);
  useEffect(() => {
    view.current?.dispatch({
      effects: language.current.reconfigure([
        sql({ dialect }),
        dialect.language.data.of({ autocomplete: completion }),
      ]),
    });
  }, [dialect, completion]);
  useEffect(() => {
    if (view.current) replaceDocument(view.current, value);
  }, [value]);
  return <div className="sql-editor" ref={element} />;
}
