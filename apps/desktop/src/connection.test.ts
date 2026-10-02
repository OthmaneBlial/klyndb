import { expect, it } from "vitest";
import {
  connectionTimeout,
  updateConnectTimeout,
  tlsSettings,
  updateTls,
} from "./connection";
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
  for (const engine of ["postgres", "mysql"]) {
    const address = updateTls(
      engine,
      `${engine}://alice@localhost/db`,
      engine === "mysql" ? "required" : "require",
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
