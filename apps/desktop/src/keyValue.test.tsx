import { renderToStaticMarkup } from "react-dom/server";
import { expect, test } from "vitest";
import { KeyReply, KeyValueWorkspace } from "./components/KeyValueWorkspace";
import { formatKeyValue, keyLabel, ttlLabel } from "./keyValue";
import { restoreWorkspace } from "./workspace";
import { updateTls, updateConnectTimeout } from "./connection";

test("Redis values remain exact escaped data and key tabs restore without persisting raw commands", () => {
  const integer = {
    kind: "cell",
    value: { kind: "number", value: "9223372036854775807" },
  } as const;
  expect(formatKeyValue(integer)).toBe("9223372036854775807");
  const html = renderToStaticMarkup(
    <KeyReply
      value={{
        kind: "array",
        value: [
          integer,
          {
            kind: "cell",
            value: { kind: "text", value: "<script>é</script>" },
          },
          { kind: "cell", value: { kind: "binary", value: "00ff" } },
        ],
      }}
    />,
  );
  expect(html).toContain("9223372036854775807");
  expect(html).toContain("&lt;script&gt;é&lt;/script&gt;");
  expect(html).not.toContain("<script>");
  expect(html).toContain("0x00ff");
  expect(keyLabel({ kind: "text", value: "" })).toBe('""');
  expect(ttlLabel("-1")).toBe("No expiry");
  expect(ttlLabel("-2")).toBe("Missing key");
  const tabs = [
    {
      id: "redis",
      name: "Keys",
      connection: "1",
      sql: "SELECT 1;",
      kind: "key_value",
    },
  ];
  expect(restoreWorkspace({ tabs }).tabs[0].kind).toBe("key_value");
  expect(
    restoreWorkspace({ tabs: [{ ...tabs[0], kind: "broken" }] }).tabs,
  ).toEqual([]);
  const workspace = renderToStaticMarkup(
    <KeyValueWorkspace
      ready={false}
      draft='["PING"]'
      onDraft={() => {}}
      onBusy={() => {}}
    />,
  );
  expect(workspace).toContain("Key explorer");
  expect(workspace).not.toContain("SQL editor");
  expect(workspace).toContain("Drafts remain in memory");
  const tls = updateTls(
    "redis",
    "rediss://alice@localhost:6379/1",
    "required",
    "/tmp/CA bundle.pem",
  );
  const url = new URL(updateConnectTimeout(tls, "45"));
  expect(url.protocol).toBe("rediss:");
  expect(url.pathname).toBe("/1");
  expect(url.searchParams.get("sslrootcert")).toBe("/tmp/CA bundle.pem");
  expect(url.searchParams.get("connect_timeout")).toBe("45");
});
