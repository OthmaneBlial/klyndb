export function tlsSettings(engine: string, address: string) {
  try {
    const url = new URL(address);
    return {
      mode:
        url.searchParams.get(engine === "mysql" ? "tls" : "sslmode") ??
        (engine === "mysql" ? "required" : "require"),
      ca: url.searchParams.get("sslrootcert") ?? "",
    };
  } catch {
    return { mode: engine === "mysql" ? "required" : "require", ca: "" };
  }
}
export function updateTls(
  engine: string,
  address: string,
  mode: string,
  ca: string,
) {
  const url = new URL(address);
  if (
    !url.hostname ||
    !["mysql:", "postgres:", "postgresql:"].includes(url.protocol)
  )
    throw new Error("Enter a valid server connection URL first.");
  url.searchParams.set(engine === "mysql" ? "tls" : "sslmode", mode);
  if (ca) url.searchParams.set("sslrootcert", ca);
  else url.searchParams.delete("sslrootcert");
  return url.toString();
}
