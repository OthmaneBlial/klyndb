use klyndb_driver_api::{Error, Result};
use tokio::{sync::Semaphore, time::Instant};
use zeroize::Zeroizing;

// ponytail: one credential reader per process; add per-key scheduling if prompts
// block unrelated lookups. OS authorization cannot be forcibly cancelled;
// retain the worker's permit after timeout to prevent blocked-thread growth.
static READS: Semaphore = Semaphore::const_new(1);

async fn lookup(
    deadline: Instant,
    read: impl FnOnce() -> Result<Option<Zeroizing<String>>> + Send + 'static,
) -> Result<Option<Zeroizing<String>>> {
    tokio::time::timeout_at(deadline, async {
        let permit = READS.acquire().await.map_err(super::error)?;
        let (send, receive) = tokio::sync::oneshot::channel();
        // A prompt may wait indefinitely; runtime shutdown must not join it.
        std::thread::Builder::new()
            .name("klyndb-credentials".into())
            .spawn(move || {
                let _permit = permit;
                let _ = send.send(read());
            })
            .map_err(super::error)?;
        receive.await.map_err(super::error)?
    })
    .await
    .map_err(|_| Error::new("OS credential lookup timed out. Handle the keychain prompt, then retry, or enter session-only credentials in Edit connection."))?
}

pub async fn password(id: &str, deadline: Instant) -> Result<Option<Zeroizing<String>>> {
    let id = id.to_owned();
    lookup(deadline, move || klyndb_connections::password(&id)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::Arc,
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    #[tokio::test(flavor = "current_thread")]
    async fn stalled_read_times_out_without_blocking_or_accumulating_workers() {
        let started = Arc::new(AtomicUsize::new(0));
        let (ready, ready_rx) = tokio::sync::oneshot::channel();
        let (release, release_rx) = std::sync::mpsc::channel();
        let count = started.clone();
        let first = lookup(Instant::now() + Duration::from_millis(20), move || {
            count.fetch_add(1, Ordering::SeqCst);
            let _ = ready.send(());
            let _ = release_rx.recv_timeout(Duration::from_secs(2));
            Ok(Some(Zeroizing::new("expired synthetic secret".into())))
        })
        .await;
        ready_rx.await.unwrap();
        let count = started.clone();
        let retry = lookup(Instant::now() + Duration::from_millis(20), move || {
            count.fetch_add(1, Ordering::SeqCst);
            Ok(None)
        })
        .await;
        let workers = started.load(Ordering::SeqCst);
        let _ = release.send(());
        assert!(first.unwrap_err().message.contains("keychain prompt"));
        assert!(retry.is_err());
        assert_eq!(workers, 1);
        // Once OS authorization finishes, a fresh read uses its own result.
        let fresh = lookup(Instant::now() + Duration::from_secs(2), || {
            Ok(Some(Zeroizing::new("fresh synthetic secret".into())))
        })
        .await
        .unwrap();
        assert_eq!(
            fresh.as_deref().map(String::as_str),
            Some("fresh synthetic secret")
        );
        let missing = lookup(Instant::now() + Duration::from_secs(2), || Ok(None))
            .await
            .unwrap();
        assert!(missing.is_none());
        let error = lookup(Instant::now() + Duration::from_secs(2), || {
            Err(Error::new("lookup failed"))
        })
        .await
        .unwrap_err();
        assert_eq!(error.message, "lookup failed");
    }
}
