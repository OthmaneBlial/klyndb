import { createRequire } from "node:module";
import { readFileSync } from "node:fs";
import { createHash } from "node:crypto";
import { execFileSync } from "node:child_process";
import { performance } from "node:perf_hooks";
import { cpus, arch, platform } from "node:os";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const require = createRequire(
  new URL("../apps/desktop/package.json", import.meta.url),
);
const { createServer } = await import(require.resolve("vite"));
const { EditorState } = await import(require.resolve("@codemirror/state"));
const { CompletionContext } = await import(
  require.resolve("@codemirror/autocomplete")
);
const { sql, PostgreSQL } = await import(
  require.resolve("@codemirror/lang-sql")
);
const { ensureSyntaxTree } = await import(
  require.resolve("@codemirror/language")
);
const server = await createServer({
  root: root + "apps/desktop",
  configFile: false,
  logLevel: "silent",
  server: { middlewareMode: true },
});
const median = (values) =>
  [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
try {
  const { tableCompletion } = await server.ssrLoadModule("/src/completion.ts");
  const { tableKey } = await server.ssrLoadModule("/src/diagram.ts");
  const cases = [];
  for (const count of [10, 1000, 10000]) {
    const users = { schema: "public", name: "users", kind: "table" };
    const tables = [
      users,
      ...Array.from({ length: count - 1 }, (_, i) => ({
        schema: "public",
        name: `table_${i}`,
        kind: "table",
      })),
    ];
    const builds = [];
    let source;
    for (let i = 0; i < 5; i++) {
      const before = performance.now();
      source = tableCompletion(
        { tables, columns: { [tableKey(users)]: ["id", "display_name"] } },
        PostgreSQL,
        async () => {
          throw new Error("Unexpected metadata request");
        },
      );
      builds.push(performance.now() - before);
    }
    const doc =
      "WITH recent AS (SELECT u.* FROM public.users u), report AS (SELECT *, 42 AS total FROM recent) SELECT * FROM report r WHERE r.";
    let state = EditorState.create({
      doc,
      extensions: [sql({ dialect: PostgreSQL })],
    });
    if (!ensureSyntaxTree(state, doc.length, 1000))
      throw new Error("Incomplete headless parse");
    state = state.update({}).state;
    const samples = [];
    for (let i = 0; i < 55; i++) {
      const before = performance.now();
      const result = await source(
        new CompletionContext(state, doc.length, true),
      );
      const elapsed = performance.now() - before;
      if (
        result?.options.map((c) => c.label).join(",") !==
        "id,display_name,total"
      )
        throw new Error("Unexpected projected columns");
      if (i >= 5) samples.push(elapsed);
    }
    cases.push({
      catalog_tables: count,
      construction_ms: builds,
      construction_median_ms: median(builds),
      warm_completion_ms: samples,
      warm_completion_median_ms: median(samples),
      warm_completion_p95_ms: [...samples].sort((a, b) => a - b)[
        Math.ceil(samples.length * 0.95) - 1
      ],
    });
  }
  const files = [
    "apps/desktop/src/completion.ts",
    "apps/desktop/src/projectedCompletion.ts",
    "apps/desktop/package-lock.json",
  ];
  console.log(
    JSON.stringify(
      {
        scope:
          "headless PostgreSQL editor completion; parsed short CTE chain; cached fixture metadata; no server/UI/render/startup measurement",
        source_base: execFileSync("git", ["rev-parse", "HEAD"], {
          cwd: root,
          encoding: "utf8",
        }).trim(),
        source_hashes: Object.fromEntries(
          files.map((f) => [
            f,
            createHash("sha256")
              .update(readFileSync(root + f))
              .digest("hex"),
          ]),
        ),
        node: process.version,
        machine: { cpu: cpus()[0]?.model, arch: arch(), platform: platform() },
        samples_per_catalog: {
          construction: 5,
          warm_completion: 50,
          warmup: 5,
        },
        cases,
      },
      null,
      2,
    ),
  );
} finally {
  await server.close();
}
