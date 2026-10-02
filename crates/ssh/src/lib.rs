use async_ssh2_lite::{
    AsyncChannel, AsyncSession, AsyncSessionStream, AsyncStream, SessionConfiguration,
    ssh2::{BlockDirections, HashType, MethodType, Session},
};
use base64::{Engine as _, engine::general_purpose::STANDARD_NO_PAD};
use klyndb_connections::ssh::Config;
use klyndb_driver_api::{Error, Result};
use std::{
    future::Future,
    net::SocketAddr,
    pin::Pin,
    task::{Context, Poll},
    time::Duration,
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    net::{TcpListener, TcpStream},
    task::{JoinHandle, JoinSet},
};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroizing;

// libssh2 reads can need writes (window adjustments). The adapter's directional assertions
// do not account for that; permit both directions for every protocol operation.
struct SshSocket(TcpStream);
#[cfg(unix)]
impl std::os::fd::AsRawFd for SshSocket {
    fn as_raw_fd(&self) -> std::os::fd::RawFd {
        self.0.as_raw_fd()
    }
}
#[cfg(windows)]
impl std::os::windows::io::AsRawSocket for SshSocket {
    fn as_raw_socket(&self) -> std::os::windows::io::RawSocket {
        self.0.as_raw_socket()
    }
}
#[async_trait::async_trait]
impl AsyncSessionStream for SshSocket {
    async fn x_with<R>(
        &self,
        op: impl FnMut() -> std::result::Result<R, async_ssh2_lite::ssh2::Error> + Send,
        session: &Session,
        _: BlockDirections,
        sleep: Option<Duration>,
    ) -> std::result::Result<R, async_ssh2_lite::Error> {
        self.0
            .x_with(op, session, BlockDirections::Both, sleep)
            .await
    }
    fn poll_x_with<R>(
        &self,
        cx: &mut std::task::Context,
        mut op: impl FnMut() -> std::io::Result<R> + Send,
        _: &Session,
        _: BlockDirections,
        _: Option<Duration>,
    ) -> std::task::Poll<std::io::Result<R>> {
        use std::{io::ErrorKind, task::Poll};
        match op() {
            Err(error) if error.kind() == ErrorKind::WouldBlock => {}
            result => return Poll::Ready(result),
        }
        // ponytail: 1 ms protocol retries; replace with shared readiness wakeups after profiling SSH idle CPU.
        let waker = cx.waker().clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(1)).await;
            waker.wake();
        });
        Poll::Pending
    }
}

// ssh2::Stream::flush discards unread incoming bytes; it is not a write flush.
// Writes already reach the transport. Preserve incoming data and send SSH EOF on shutdown.
type SendEof<'a> =
    Pin<Box<dyn Future<Output = std::result::Result<(), async_ssh2_lite::Error>> + Send + 'a>>;
struct ForwardStream<'a> {
    stream: AsyncStream<SshSocket>,
    eof: Option<SendEof<'a>>,
}
impl<'a> ForwardStream<'a> {
    fn new(channel: &'a mut AsyncChannel<SshSocket>) -> Self {
        Self {
            stream: channel.stream(0),
            eof: Some(Box::pin(channel.send_eof())),
        }
    }
}
impl AsyncRead for ForwardStream<'_> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(cx, buf)
    }
}
impl AsyncWrite for ForwardStream<'_> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        let Some(eof) = self.eof.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let result = std::task::ready!(eof.as_mut().poll(cx));
        self.eof = None;
        Poll::Ready(result.map_err(std::io::Error::other))
    }
}

fn failure() -> Error {
    Error::new("SSH connection failed. Check host, verified fingerprint and authentication.")
}

pub struct Tunnel {
    pub endpoint: SocketAddr,
    stop: CancellationToken,
    task: JoinHandle<()>,
}
impl Tunnel {
    pub async fn open(
        config: Config,
        target: String,
        port: u16,
        password: Option<&str>,
        timeout: Duration,
    ) -> Result<Self> {
        klyndb_connections::ssh::validate_password(password.unwrap_or_default())?;
        let stream = TcpStream::connect((config.host.as_str(), config.port))
            .await
            .map_err(|_| failure())?;
        stream.set_nodelay(true).map_err(|_| failure())?;
        let mut settings = SessionConfiguration::new();
        settings.set_keepalive(true, 30);
        let mut handle = AsyncSession::new(SshSocket(stream), settings).map_err(|_| failure())?;
        // Exclude SHA-1 host signatures, weak DH groups and CBC ciphers before negotiation.
        for (method, preferences) in [
            (
                MethodType::HostKey,
                "ssh-ed25519,ecdsa-sha2-nistp256,ecdsa-sha2-nistp384,ecdsa-sha2-nistp521,rsa-sha2-512,rsa-sha2-256",
            ),
            (
                MethodType::Kex,
                "curve25519-sha256,curve25519-sha256@libssh.org,ecdh-sha2-nistp256,ecdh-sha2-nistp384,ecdh-sha2-nistp521,diffie-hellman-group16-sha512,diffie-hellman-group14-sha256",
            ),
            (
                MethodType::CryptCs,
                "aes256-gcm@openssh.com,aes128-gcm@openssh.com,aes256-ctr,aes128-ctr",
            ),
            (
                MethodType::CryptSc,
                "aes256-gcm@openssh.com,aes128-gcm@openssh.com,aes256-ctr,aes128-ctr",
            ),
        ] {
            handle
                .method_pref(method, preferences)
                .await
                .map_err(|_| failure())?;
        }
        handle.handshake().await.map_err(|_| failure())?;
        // Compare the presented raw host key before sending any credential or agent signature.
        let hash = handle.host_key_hash(HashType::Sha256).ok_or_else(failure)?;
        if format!("SHA256:{}", STANDARD_NO_PAD.encode(hash)) != config.fingerprint {
            return Err(Error::new(
                "SSH host key fingerprint mismatch. Verify the host key with your administrator.",
            ));
        }
        match config.auth.as_str() {
            "password" => handle
                .userauth_password(&config.user, password.unwrap_or_default())
                .await
                .map_err(|_| failure())?,
            "key" => {
                let path = config.identity.as_deref().ok_or_else(failure)?;
                let bytes =
                    klyndb_driver_api::tls::read_security_file(path, "SSH identity").await?;
                let password = password.map(|p| Zeroizing::new(p.to_owned()));
                let authentication = handle.clone();
                let runtime = tokio::runtime::Handle::current();
                tokio::task::spawn_blocking(move || {
                    let text = std::str::from_utf8(&bytes).map_err(|_| failure())?;
                    runtime.block_on(authentication.userauth_pubkey_memory(&config.user, None, text, password.as_deref().map(|p| p.as_str())))
                        .map_err(|_| Error::new("SSH private-key authentication failed. Check the key and passphrase."))
                }).await.map_err(|_| failure())??;
            }
            "agent" => {
                let mut agent = handle.agent().map_err(|_| failure())?;
                agent.connect().await.map_err(|_| {
                    Error::new("SSH agent is unavailable. Start an agent or choose a private key.")
                })?;
                agent.list_identities().await.map_err(|_| failure())?;
                let keys = agent.identities().map_err(|_| failure())?;
                if keys.len() > 32 {
                    return Err(Error::new(
                        "Too many SSH agent identities. Choose a private key.",
                    ));
                }
                for key in keys {
                    if agent.userauth(&config.user, &key).await.is_ok() && handle.authenticated() {
                        break;
                    }
                }
                let _ = agent.disconnect().await;
            }
            _ => return Err(failure()),
        }
        if !handle.authenticated() {
            return Err(failure());
        }
        // Bind an owned listener atomically; no port reservation race or subprocess is involved.
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .map_err(|_| failure())?;
        let endpoint = listener.local_addr().map_err(|_| failure())?;
        let stop = CancellationToken::new();
        let stopping = stop.clone();
        let task = tokio::spawn(async move {
            let mut channels = JoinSet::new();
            let mut keepalive = tokio::time::interval(Duration::from_secs(30));
            loop {
                tokio::select! {
                    biased;
                    _ = stopping.cancelled() => break,
                    _ = channels.join_next(), if !channels.is_empty() => {},
                    _ = keepalive.tick() => {
                        let sent = tokio::select! {
                            _ = stopping.cancelled() => break,
                            sent = tokio::time::timeout(timeout, handle.keepalive_send()) => sent,
                        };
                        if !matches!(sent, Ok(Ok(_))) { break; }
                    },
                    accepted = listener.accept() => {
                        let Ok((mut socket, source)) = accepted else { break; };
                        // The user session and cancellation need separate channels; reject excess local clients.
                        if channels.len() >= 4 { continue; }
                        let source_host = source.ip().to_string();
                        let opened = tokio::select! {
                            _ = stopping.cancelled() => break,
                            channel = tokio::time::timeout(timeout, handle.channel_direct_tcpip(&target, port, Some((&source_host, source.port())))) => channel,
                        };
                        if let Ok(Ok(mut channel)) = opened {
                            channels.spawn(async move {
                                let _ = tokio::io::copy_bidirectional(&mut socket, &mut ForwardStream::new(&mut channel)).await;
                                let _ = channel.close().await;
                            });
                        }
                    }
                }
            }
            drop(listener);
            channels.abort_all();
            while channels.join_next().await.is_some() {}
            let _ = tokio::time::timeout(Duration::from_secs(1), handle.disconnect(None, "", None))
                .await;
        });
        Ok(Self {
            endpoint,
            stop,
            task,
        })
    }
    pub async fn close(&mut self) {
        self.stop.cancel();
        if tokio::time::timeout(Duration::from_secs(2), &mut self.task)
            .await
            .is_err()
        {
            self.task.abort();
        }
    }
}
impl Drop for Tunnel {
    fn drop(&mut self) {
        self.stop.cancel();
    }
}
