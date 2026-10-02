export function tlsSettings(engine: string, address: string) {
  try {
    const url = new URL(address);
    return {
      mode:
        url.searchParams.get(engine === "mysql" ? "tls" : "sslmode") ??
        (engine === "mysql" ? "required" : "require"),
      ca: url.searchParams.get("sslrootcert") ?? "",
      identity: url.searchParams.get("sslidentity") ?? "",
    };
  } catch {
    return {
      mode: engine === "mysql" ? "required" : "require",
      ca: "",
      identity: "",
    };
  }
}
function serverUrl(address: string) {
  const url = new URL(address);
  if (
    !url.hostname ||
    !["mysql:", "postgres:", "postgresql:"].includes(url.protocol)
  )
    throw new Error("Enter a valid server connection URL first.");
  return url;
}
const sshOptions = [
  "host",
  "port",
  "user",
  "auth",
  "identity",
  "fingerprint",
] as const;
export type SshSettings = {
  enabled: boolean;
  host: string;
  port: string;
  user: string;
  auth: string;
  identity: string;
  fingerprint: string;
};
export function sshSettings(address: string): SshSettings {
  const defaults = {
    enabled: false,
    host: "",
    port: "22",
    user: "",
    auth: "agent",
    identity: "",
    fingerprint: "",
  };
  try {
    const options = new URL(address).searchParams;
    return {
      enabled: options.has("ssh_host"),
      host: options.get("ssh_host") ?? "",
      port: options.get("ssh_port") ?? "22",
      user: options.get("ssh_user") ?? "",
      auth: options.get("ssh_auth") ?? "agent",
      identity: options.get("ssh_identity") ?? "",
      fingerprint: options.get("ssh_fingerprint") ?? "",
    };
  } catch {
    return defaults;
  }
}
export function updateSsh(address: string, settings: SshSettings) {
  const url = serverUrl(address);
  for (const option of sshOptions) {
    if (settings.enabled && (option !== "identity" || settings.auth === "key"))
      url.searchParams.set(`ssh_${option}`, settings[option]);
    else url.searchParams.delete(`ssh_${option}`);
  }
  return url.toString();
}
export function connectionTimeout(address: string) {
  try {
    return new URL(address).searchParams.get("connect_timeout") ?? "10";
  } catch {
    return "10";
  }
}
export function updateConnectTimeout(address: string, seconds: string) {
  const url = serverUrl(address);
  url.searchParams.set("connect_timeout", seconds);
  return url.toString();
}
export function updateTls(
  engine: string,
  address: string,
  mode: string,
  ca: string,
  identity?: string,
) {
  const url = serverUrl(address);
  url.searchParams.set(engine === "mysql" ? "tls" : "sslmode", mode);
  if (ca) url.searchParams.set("sslrootcert", ca);
  else url.searchParams.delete("sslrootcert");
  if (identity !== undefined) {
    if (identity) url.searchParams.set("sslidentity", identity);
    else url.searchParams.delete("sslidentity");
  }
  return url.toString();
}
