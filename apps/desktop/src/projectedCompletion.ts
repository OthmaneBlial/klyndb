import type {
  CompletionContext,
  CompletionResult,
} from "@codemirror/autocomplete";
import { syntaxTree } from "@codemirror/language";
import type { SQLDialect } from "@codemirror/lang-sql";
import type { Table } from "./api";

type SyntaxNode = ReturnType<typeof syntaxTree>["topNode"];
type Token = Pick<SyntaxNode, "name" | "from" | "to" | "firstChild">;

type Name = { value: string; quoted: boolean };
type Relation = { columns: () => Promise<string[]> };
type Scope = {
  ctes: Map<string, Relation>;
  bodies: Map<number, Map<string, Relation>>;
  sources: Map<string, Relation>;
  rows: Relation[];
};

// Infer output names from the existing editor tree, never by executing SQL.
export async function projectedCompletion(
  context: CompletionContext,
  dialect: SQLDialect,
  tables: Table[],
  load: (table: Table) => Promise<string[]>,
): Promise<{ qualified: boolean; result: CompletionResult } | null> {
  const doc = context.state.doc;
  const text = (node: Token) => doc.sliceString(node.from, node.to);
  const children = (node: Token) => {
    const nodes: Token[] = [];
    for (let child = node.firstChild; child; child = child.nextSibling)
      if (
        !/Comment/.test(child.name) &&
        child.name !== "(" &&
        child.name !== ")"
      ) {
        const previous = nodes.at(-1);
        if (
          child.name === "QuotedIdentifier" &&
          previous?.name === "QuotedIdentifier" &&
          previous.to === child.from &&
          text(previous)[0] === text(child)[0]
        )
          nodes[nodes.length - 1] = {
            name: child.name,
            from: previous.from,
            to: child.to,
            firstChild: null,
          };
        else nodes.push(child);
      }
    return nodes;
  };
  const word = (node?: Token) => (node ? text(node).toLowerCase() : "");
  const name = (node?: Token): Name | null => {
    if (
      !node ||
      !/^(Identifier|QuotedIdentifier|Keyword|Builtin|Type)$/.test(node.name)
    )
      return null;
    const value = text(node);
    const quoted = node.name === "QuotedIdentifier";
    return {
      value: quoted
        ? value.slice(1, -1).replaceAll(value.at(-1)!.repeat(2), value.at(-1)!)
        : value,
      quoted,
    };
  };
  const key = (n: Name) =>
    n.quoted && !dialect.spec.caseInsensitiveIdentifiers
      ? n.value
      : n.value.toLowerCase();
  const path = (node: Token): Name[] =>
    node.name === "CompositeIdentifier"
      ? children(node).flatMap((n) => name(n) ?? [])
      : name(node)
        ? [name(node)!]
        : [];
  const isQuery = (node: Token) =>
    children(node).some((n) => word(n) === "select");
  const list = (node: Token) =>
    children(node).flatMap((n) => (name(n) ? key(name(n)!) : []));
  const scopes = new Map<number, Scope>();
  const stop = new Set([
    "where",
    "group",
    "having",
    "order",
    "union",
    "intersect",
    "except",
    "limit",
    "offset",
    "fetch",
    "for",
    "window",
    "returning",
  ]);
  const empty: Relation = { columns: async () => [] };

  function table(path: Name[], ctes: Map<string, Relation>): Relation {
    if (path.length === 1 && ctes.has(key(path[0])))
      return ctes.get(key(path[0]))!;
    const matches = tables.filter((t) => {
      const actual = t.schema ? [t.schema, t.name] : [t.name];
      const target = path.length === 1 ? [t.name] : actual;
      return (
        target.length === path.length &&
        target.every(
          (v, i) =>
            (dialect.spec.caseInsensitiveIdentifiers ? v.toLowerCase() : v) ===
            key(path[i]),
        )
      );
    });
    return matches.length === 1 ? { columns: () => load(matches[0]) } : empty;
  }

  function body(
    node: Token,
    ctes: Map<string, Relation>,
    depth: number,
  ): Relation {
    let busy = false;
    return {
      columns: async () => {
        if (busy || depth > 32 || context.aborted) return [];
        busy = true;
        try {
          return await projection(node, scope(node, ctes, depth + 1));
        } finally {
          busy = false;
        }
      },
    };
  }

  function scope(
    node: Token,
    inherited: Map<string, Relation>,
    depth: number,
  ): Scope {
    const existing = scopes.get(node.from);
    if (existing) return existing;
    const s: Scope = {
      ctes: new Map(inherited),
      bodies: new Map(),
      sources: new Map(),
      rows: [],
    };
    scopes.set(node.from, s);
    const nodes = children(node);
    let i = 0;
    if (word(nodes[i]) === "with") {
      i++;
      const recursive = word(nodes[i]) === "recursive";
      if (recursive) i++;
      while (i < nodes.length) {
        const id = name(nodes[i++]);
        if (!id) break;
        let explicit: string[] | null = null;
        if (nodes[i]?.name === "Parens") explicit = list(nodes[i++]);
        if (word(nodes[i++]) !== "as") break;
        if (word(nodes[i]) === "not") i++;
        if (word(nodes[i]) === "materialized") i++;
        const query = nodes[i++];
        if (query?.name !== "Parens") break;
        const visible = new Map(s.ctes);
        const relation = explicit
          ? { columns: async () => explicit! }
          : body(query, visible, depth);
        if (recursive) visible.set(key(id), relation);
        s.bodies.set(query.from, visible);
        s.ctes.set(key(id), relation);
        if (word(nodes[i]) !== ",") break;
        i++;
      }
    }
    i = nodes.findIndex((n) => word(n) === "from");
    if (i < 0) return s;
    let expectSource = true;
    for (i++; i < nodes.length; i++) {
      const n = nodes[i],
        w = word(n);
      if (stop.has(w)) break;
      if (w === "join" || w === ",") {
        expectSource = true;
        continue;
      }
      if (!expectSource || ["lateral", "only"].includes(w)) continue;
      expectSource = false;
      const p = path(n);
      let relation =
        n.name === "Parens" && isQuery(n)
          ? body(n, s.ctes, depth)
          : table(p, s.ctes);
      let alias = p.at(-1) ?? null;
      const explicitAlias = word(nodes[i + 1]) === "as";
      if (explicitAlias) i++;
      const candidate = nodes[i + 1];
      if (
        candidate &&
        (candidate.name === "Identifier" ||
          candidate.name === "QuotedIdentifier" ||
          (explicitAlias && name(candidate)))
      ) {
        alias = name(candidate);
        i++;
      }
      if (nodes[i + 1]?.name === "Parens" && alias) {
        const renamed = list(nodes[++i]);
        const original = relation;
        relation = {
          columns: async () => {
            const cols = await original.columns();
            return cols.map((col, index) => renamed[index] ?? col);
          },
        };
      }
      if (alias) s.sources.set(key(alias), relation);
      if (p.length > 1) s.sources.set(p.map(key).join("\0"), relation);
      s.rows.push(relation);
    }
    return s;
  }

  async function projection(node: Token, s: Scope): Promise<string[]> {
    const nodes = children(node);
    let start = nodes.findIndex((n) => word(n) === "select") + 1;
    if (!start) return [];
    if (["distinct", "all"].includes(word(nodes[start]))) start++;
    if (word(nodes[start]) === "on" && nodes[start + 1]?.name === "Parens")
      start += 2;
    const end = nodes.findIndex(
      (n, i) => i >= start && (word(n) === "from" || stop.has(word(n))),
    );
    const items: Token[][] = [[]];
    for (const n of nodes.slice(start, end < 0 ? undefined : end)) {
      if (word(n) === ",") items.push([]);
      else items.at(-1)!.push(n);
    }
    const output: string[] = [];
    for (const item of items) {
      const last = item.at(-1),
        before = item.at(-2);
      const alias = name(last);
      if (
        alias &&
        (word(before) === "as" ||
          (item.length > 1 &&
            before!.to < last!.from &&
            before!.name !== "Operator"))
      ) {
        output.push(key(alias));
      } else if (item.length === 1 && last) {
        if (word(last) === "*") {
          for (const source of s.rows) output.push(...(await source.columns()));
        } else {
          const p = path(last);
          if (p.length) output.push(key(p.at(-1)!));
        }
      } else if (
        item.length === 2 &&
        word(last) === "*" &&
        item[0].name === "CompositeIdentifier"
      ) {
        const p = path(item[0]);
        const source = s.sources.get(p.map(key).join("\0")) ?? table(p, s.ctes);
        output.push(...(await source.columns()));
      }
    }
    return output;
  }

  const tree = syntaxTree(context.state);
  let at = tree.resolveInner(context.pos, -1);
  if (at.name === "Script") {
    const previous = tree.topNode.childBefore(context.pos);
    if (
      previous?.name === "Statement" &&
      !text(previous).trimEnd().endsWith(";") &&
      !doc.sliceString(previous.to, context.pos).trim()
    )
      at = previous;
  }
  const queries: SyntaxNode[] = [];
  for (let n: SyntaxNode | null = at; n; n = n.parent) {
    if (n.name === "Statement" || (n.name === "Parens" && isQuery(n)))
      queries.unshift(n);
    if (n.name === "Statement") break;
  }
  // ponytail: bound local inference to 64 KiB/32 scopes. Larger scripts retain
  // catalog completion; expand only with measured editor latency evidence.
  if (
    !queries.length ||
    queries[0].to - queries[0].from > 65536 ||
    queries.length > 32
  )
    return null;
  let s = scope(queries[0], new Map(), 0);
  for (let i = 1; i < queries.length; i++)
    s = scope(queries[i], s.bodies.get(queries[i].from) ?? s.ctes, i);
  const local =
    s.ctes.size ||
    queries.length > 1 ||
    children(queries[0]).some((n) => n.name === "Parens" && isQuery(n));
  if (!local) return null;

  let from = context.pos,
    quoted = "",
    parents: Name[] = [];
  if (
    ["Identifier", "QuotedIdentifier", "Keyword", "Builtin", "Type"].includes(
      at.name,
    )
  ) {
    from = at.from;
    if (at.name === "QuotedIdentifier") quoted = text(at)[0];
    if (at.parent?.name === "CompositeIdentifier")
      parents = children(at.parent)
        .filter((n) => n.to <= at.from)
        .flatMap((n) => name(n) ?? []);
  } else if (at.name === "." && at.parent?.name === "CompositeIdentifier") {
    parents = children(at.parent)
      .filter((n) => n.to <= at.from)
      .flatMap((n) => name(n) ?? []);
  }
  const closing = quoted === "[" ? "]" : quoted;
  const result = (names: string[], type: string): CompletionResult => ({
    from,
    to:
      quoted && doc.sliceString(context.pos, context.pos + 1) === closing
        ? context.pos + 1
        : undefined,
    options: [...new Set(names)].map((label) => {
      const quote = quoted || dialect.spec.identifierQuotes?.[0] || '"';
      const close = quote === "[" ? "]" : quote;
      const escaped = label.replaceAll(close, close + close);
      return quoted
        ? { label: quote + escaped + close, type }
        : {
            label,
            type,
            apply: new RegExp(
              "^[a-z_][a-z_\\d]*$",
              dialect.spec.caseInsensitiveIdentifiers ? "i" : "",
            ).test(label)
              ? undefined
              : quote + escaped + close,
          };
    }),
    validFor: quoted
      ? /^[`"[]?[\p{L}\p{N}_$ ]*[`"\]]?$/u
      : /^[\p{L}\p{N}_$]*$/u,
  });
  if (parents.length) {
    const relation =
      s.sources.get(parents.map(key).join("\0")) ??
      (parents.length === 1 ? s.ctes.get(key(parents[0])) : undefined);
    return relation
      ? {
          qualified: true,
          result: result(await relation.columns(), "property"),
        }
      : null;
  }
  if (!context.explicit && from === context.pos) return null;
  return { qualified: false, result: result([...s.ctes.keys()], "type") };
}
