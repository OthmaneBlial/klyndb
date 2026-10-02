use klyndb_connections::ssh::Config;
use klyndb_ssh::Tunnel;
use russh::{
    Channel,
    server::{self, Auth, ChannelOpenHandle, Msg, Session},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    task::JoinSet,
};

const PASSWORD: &str = "synthetic-ssh-test-password";
struct Bastion {
    attempts: Arc<AtomicUsize>,
    target: std::net::SocketAddr,
    channels: JoinSet<()>,
}
impl server::Handler for Bastion {
    type Error = russh::Error;
    async fn auth_password(&mut self, user: &str, password: &str) -> Result<Auth, Self::Error> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Ok(if user == "test" && password == PASSWORD {
            Auth::Accept
        } else {
            Auth::reject()
        })
    }
    async fn channel_open_direct_tcpip(
        &mut self,
        channel: Channel<Msg>,
        host: &str,
        port: u32,
        _: &str,
        _: u32,
        reply: ChannelOpenHandle,
        _: &mut Session,
    ) -> Result<(), Self::Error> {
        if host != "127.0.0.1" || port != u32::from(self.target.port()) {
            return Ok(());
        }
        let mut socket = TcpStream::connect(self.target).await?;
        socket.set_nodelay(true)?;
        reply.accept().await;
        self.channels.spawn(async move {
            let mut stream = channel.into_stream();
            let _ = tokio::io::copy_bidirectional(&mut socket, &mut stream).await;
        });
        Ok(())
    }
}

#[tokio::test]
async fn password_authentication_pins_before_credentials_forwards_and_cleans_up() {
    // Independent real SSH server plus a TCP echo endpoint; no database success is mocked.
    let echo = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let target = echo.local_addr().unwrap();
    let delivered = Arc::new(AtomicUsize::new(0));
    let counter = delivered.clone();
    let echo_task = tokio::spawn(async move {
        let mut tasks = JoinSet::new();
        loop {
            let (mut socket, _) = echo.accept().await.unwrap();
            socket.set_nodelay(true).unwrap();
            let counter = counter.clone();
            tasks.spawn(async move {
                let mut buf = [0; 8192];
                while let Ok(size) = socket.read(&mut buf).await {
                    if size == 0 {
                        break;
                    }
                    counter.fetch_add(size, Ordering::SeqCst);
                    if socket.write_all(&buf[..size]).await.is_err() {
                        break;
                    }
                }
            });
        }
    });
    let key =
        russh::keys::PrivateKey::random(&mut rand::rng(), russh::keys::Algorithm::Ed25519).unwrap();
    let pin = key
        .public_key()
        .fingerprint(russh::keys::ssh_key::HashAlg::Sha256)
        .to_string();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = listener.local_addr().unwrap();
    let attempts = Arc::new(AtomicUsize::new(0));
    let seen = attempts.clone();
    let server_task = tokio::spawn(async move {
        let config = Arc::new(server::Config {
            keys: vec![key],
            auth_rejection_time: Duration::from_millis(10),
            ..Default::default()
        });
        let mut sessions = JoinSet::new();
        loop {
            let (socket, _) = listener.accept().await.unwrap();
            socket.set_nodelay(true).unwrap();
            let config = config.clone();
            let handler = Bastion {
                attempts: seen.clone(),
                target,
                channels: JoinSet::new(),
            };
            sessions.spawn(async move {
                if let Ok(session) = server::run_stream(config, socket, handler).await {
                    let _ = session.await;
                }
            });
        }
    });
    let make = |fingerprint| Config {
        host: "127.0.0.1".into(),
        port: endpoint.port(),
        user: "test".into(),
        auth: "password".into(),
        identity: None,
        fingerprint,
    };
    let open = |config, password| {
        Tunnel::open(
            config,
            "127.0.0.1".into(),
            target.port(),
            password,
            Duration::from_secs(3),
        )
    };
    let timeout = Duration::from_secs(5);
    let rejected = tokio::time::timeout(
        timeout,
        open(make(format!("SHA256:{}", "A".repeat(43))), Some(PASSWORD)),
    )
    .await
    .unwrap();
    assert!(
        rejected
            .err()
            .unwrap()
            .message
            .contains("fingerprint mismatch")
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 0);
    assert!(
        tokio::time::timeout(timeout, open(make(pin.clone()), Some("wrong-password")))
            .await
            .unwrap()
            .is_err()
    );
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let mut tunnel = tokio::time::timeout(timeout, open(make(pin.clone()), Some(PASSWORD)))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
    let mut socket = TcpStream::connect(tunnel.endpoint).await.unwrap();
    // More than one SSH window, in both directions, verifies forwarding and backpressure.
    let payload: Vec<u8> = (0..2 * 1024 * 1024).map(|i| (i % 251) as u8).collect();
    let (mut read, mut write) = socket.split();
    let mut received = vec![0; payload.len()];
    let written = AtomicUsize::new(0);
    let read_bytes = AtomicUsize::new(0);
    // This is a correctness guard, not the product streaming benchmark.
    let transferred = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::try_join!(
            async {
                for chunk in payload.chunks(8192) {
                    write.write_all(chunk).await?;
                    written.fetch_add(chunk.len(), Ordering::SeqCst);
                }
                std::io::Result::Ok(())
            },
            async {
                let mut pos = 0;
                while pos < received.len() {
                    let bytes = read.read(&mut received[pos..]).await?;
                    if bytes == 0 {
                        return Err(std::io::ErrorKind::UnexpectedEof.into());
                    }
                    pos += bytes;
                    read_bytes.store(pos, Ordering::SeqCst);
                }
                std::io::Result::Ok(())
            }
        )
        .unwrap();
    })
    .await;
    assert!(
        transferred.is_ok(),
        "written={}, received={}, server_received={}",
        written.load(Ordering::SeqCst),
        read_bytes.load(Ordering::SeqCst),
        delivered.load(Ordering::SeqCst)
    );
    assert_eq!(received, payload);
    // A second simultaneous channel must drain its reply after the local write half closes.
    let mut half_closed = TcpStream::connect(tunnel.endpoint).await.unwrap();
    let more: Vec<u8> = (0..3 * 1024 * 1024).map(|i| (i % 239) as u8).collect();
    let mut reply = vec![0; more.len()];
    let (mut reader, mut writer) = half_closed.split();
    tokio::time::timeout(Duration::from_secs(15), async {
        tokio::try_join!(
            async {
                for chunk in more.chunks(8192) {
                    writer.write_all(chunk).await?;
                }
                writer.shutdown().await
            },
            async {
                reader.read_exact(&mut reply).await?;
                assert_eq!(reader.read(&mut [0]).await?, 0);
                std::io::Result::Ok(())
            }
        )
        .unwrap();
    })
    .await
    .unwrap();
    assert_eq!(reply, more);
    let port = tunnel.endpoint;
    tunnel.close().await;
    assert!(TcpStream::connect(port).await.is_err());
    assert_eq!(
        tokio::time::timeout(timeout, socket.read(&mut [0]))
            .await
            .unwrap()
            .unwrap(),
        0
    );
    let tunnel = tokio::time::timeout(timeout, open(make(pin), Some(PASSWORD)))
        .await
        .unwrap()
        .unwrap();
    let port = tunnel.endpoint;
    drop(tunnel);
    tokio::time::timeout(timeout, async {
        while TcpStream::connect(port).await.is_ok() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    echo_task.abort();
    server_task.abort();
}
