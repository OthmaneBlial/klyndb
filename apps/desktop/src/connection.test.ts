import { expect, it } from "vitest";
import { tlsSettings, updateTls } from "./connection";
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
