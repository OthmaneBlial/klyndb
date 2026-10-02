# SSH tunnels and bastions

PostgreSQL, MySQL and MariaDB can connect through one SSH server. Klyndb forwards native TCP channels through libssh2; it does not launch a shell or copy private keys into application state.

## Connect through a bastion

1. Enter the database URL using the hostname and port reachable **from the SSH server**.
2. Expand **SSH tunnel**, enable it, and enter the SSH host, port and username.
3. Enter the server's **SHA256 host-key fingerprint**, obtained through a trusted administrator or other independent channel. Do not trust an unverified fingerprint returned by the network you are trying to verify.
4. Choose **SSH agent**, **Private key**, or **SSH password**. The native key picker accepts an absolute OpenSSH or supported PEM key path. Protected keys use the separate masked SSH passphrase field.
5. Choose **Test connection**, then save and connect.

The SSH host key is checked before any password, private-key authentication or agent signature is sent. A mismatch fails closed; verify a legitimate host-key rotation before updating the saved fingerprint. There is no automatic trust-on-first-use.

Database TLS remains independent of SSH. Keep verified TLS enabled and use the database certificate's hostname in the database URL. The connectors use the local forwarding endpoint for TCP while retaining the original hostname for TLS verification. Custom CAs and PKCS#12 client identities work through the tunnel. An SSH connection alone does not prove database TLS is enabled.

## Credentials and files

Database passwords, TLS archive passwords and SSH passwords/passphrases have separate OS keychain entries. Clear **Store SSH secret in the OS keychain** to use it only for the immediate Save & connect. For a protected key without a stored passphrase, reopen Edit connection and enter the passphrase when reconnecting. Leaving the field empty uses the saved secret only when the saved SSH settings still match. Changing the bastion, username, authentication, key path or fingerprint does not reuse the old stored secret during a draft test. Saving changed settings without a replacement clears the old entry; removing SSH or deleting the connection also clears it. Duplicating a connection copies paths/settings, without copying credentials.

Private-key files must be nonempty regular files up to 1 MiB. Rust reads them on each connection attempt; keep the file available and protected. Klyndb-managed key/password buffers are zeroized when dropped. Native SSH/OpenSSL copies follow their library lifecycle. Use keys from trusted sources: the connection deadline bounds waiting for native key parsing, but cannot forcibly terminate a blocking native parser.

Agent authentication uses the local agent without forwarding it to the bastion. It tries at most 32 identities. The Windows native provider supports OpenSSH-agent/Pageant transports; actual Windows/Linux workflows still need validation.

## URL settings

Non-secret SSH options are `ssh_host`, `ssh_port` (default 22), `ssh_user`, `ssh_auth` (`agent`, `key`, `password`), `ssh_identity` (only for key authentication), and `ssh_fingerprint` (`SHA256:` plus 43 unpadded base64 characters). URL-encode paths and the fingerprint. Password/passphrase URL options, duplicate/unknown SSH options and conflicting PostgreSQL `host`/`hostaddr`/`port` query routing are rejected. The URL authority identifies the database destination.

The Network connection timeout (1–300 seconds, default 10) covers SSH plus database setup. Keepalive runs every 30 seconds. Each tunnel listens only on an atomically assigned loopback port and permits up to four live local channels, including database cancellation. Disconnect closes its listener and channels; failed attempts and dropped sessions cancel their tunnel.

## Validation and current limits

Real OpenSSH contracts cover PostgreSQL, MySQL and MariaDB with database mutual TLS, verified host keys, agent and Ed25519/RSA/ECDSA key authentication, queries, metadata, 2 MiB results, cancellation and reconnect. An independent SSH server checks password authentication, rejection before credentials, simultaneous channels, sustained bidirectional data, half-close draining and tunnel teardown. See [validation evidence](VALIDATION.md).

One bastion is supported. Multi-hop ProxyJump, proxies, SSH host certificates, automatic known_hosts import, keyboard-interactive/MFA and hardware-key UX remain pending. Native Windows/Linux and broader key-format validation remain pending. The current async provider retries pending channel operations every millisecond; idle SSH CPU and larger concurrent workloads still need measurement and readiness-based optimization before performance claims.
