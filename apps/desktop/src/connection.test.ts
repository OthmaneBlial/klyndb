import { expect, it } from "vitest";
import {
  connectionTimeout,
  updateConnectTimeout,
  tlsSettings,
  updateTls,
  sshSettings,
  updateSsh,
} from "./connection";
it("round-trips SSH settings without losing database TLS and removes unused key paths", () => {
  const original =
    "postgresql://alice@db.internal/db?sslmode=require&sslrootcert=%2Ftmp%2Fca.pem&connect_timeout=45";
  const settings = {
    ...sshSettings(original),
    enabled: true,
    host: "bastion.example.com",
    user: "alice",
    auth: "key",
    identity: "/tmp/private key",
    fingerprint: `SHA256:${"A".repeat(43)}`,
  };
  const address = updateSsh(original, settings);
  expect(sshSettings(address)).toEqual(settings);
  expect(tlsSettings("postgres", address)).toEqual(
    tlsSettings("postgres", original),
  );
  expect(connectionTimeout(address)).toBe("45");
  const agent = updateSsh(address, { ...settings, auth: "agent" });
  expect(new URL(agent).searchParams.has("ssh_identity")).toBe(false);
  expect(updateSsh(address, { ...settings, enabled: false })).toBe(
    new URL(original).toString(),
  );
  expect(() => updateSsh("sqlite:///tmp/db", settings)).toThrow();
});
it("round-trips TLS certificate paths without dropping connection options", () => {
  const ca = "/tmp/private database/ca.pem";
  const address = updateTls(
    "postgres",
    "postgresql://alice@localhost/db?application_name=Klyndb&sslmode=disable",
    "require",
    ca,
  );
  expect(tlsSettings("postgres", address)).toEqual({
    mode: "require",
    ca,
    identity: "",
  });
  expect(new URL(address).searchParams.get("application_name")).toBe("Klyndb");
  expect(
    new URL(updateTls("postgres", address, "require", "")).searchParams.has(
      "sslrootcert",
    ),
  ).toBe(false);
  expect(
    tlsSettings(
      "mysql",
      updateTls("mysql", "mysql://alice@localhost/db", "required", ca),
    ),
  ).toEqual({ mode: "required", ca, identity: "" });
  const withIdentity = updateTls(
    "postgres",
    address,
    "require",
    ca,
    "/tmp/identity.p12",
  );
  expect(tlsSettings("postgres", withIdentity).identity).toBe(
    "/tmp/identity.p12",
  );
  expect(
    tlsSettings("postgres", updateTls("postgres", withIdentity, "require", ""))
      .identity,
  ).toBe("/tmp/identity.p12");
  expect(
    tlsSettings(
      "postgres",
      updateTls("postgres", withIdentity, "require", ca, ""),
    ).identity,
  ).toBe("");
  expect(() => updateTls("postgres", "not a URL", "require", ca)).toThrow();
});
it("round-trips a network deadline while preserving TLS identity and other options", () => {
  for (const engine of ["postgres", "mysql", "clickhouse", "mssql"]) {
    const address = updateTls(
      engine,
      `${engine}://alice@localhost/db`,
      engine !== "postgres" ? "required" : "require",
      "/tmp/ca.pem",
      "/tmp/client.p12",
    );
    expect(connectionTimeout(address)).toBe("10");
    const changed = updateConnectTimeout(address, "45");
    expect(connectionTimeout(changed)).toBe("45");
    expect(tlsSettings(engine, changed)).toEqual(tlsSettings(engine, address));
    expect(connectionTimeout(updateConnectTimeout(changed, ""))).toBe("");
  }
  expect(() => updateConnectTimeout("sqlite:///file.db", "30")).toThrow();
});
