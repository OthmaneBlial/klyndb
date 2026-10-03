use std::{
    io,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

pub(crate) const RESPONSE_BYTES: usize = 8 * 1024 * 1024;
pub(crate) struct Bounded<S> {
    pub stream: S,
    pub remaining: Arc<AtomicUsize>,
}
impl<S: AsyncRead + Unpin> AsyncRead for Bounded<S> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        let available = this.remaining.load(Ordering::Relaxed).min(buf.remaining());
        if buf.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        if available == 0 {
            return Poll::Ready(Err(io::Error::other(
                "Redis response exceeds the transport limit",
            )));
        }
        let mut limited = ReadBuf::new(&mut buf.initialize_unfilled()[..available]);
        match Pin::new(&mut this.stream).poll_read(cx, &mut limited) {
            Poll::Ready(Ok(())) => {
                let read = limited.filled().len();
                this.remaining.fetch_sub(read, Ordering::Relaxed);
                buf.advance(read);
                Poll::Ready(Ok(()))
            }
            result => result,
        }
    }
}
impl<S: AsyncWrite + Unpin> AsyncWrite for Bounded<S> {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().stream).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().stream).poll_shutdown(cx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    #[tokio::test]
    async fn stops_before_unbounded_reply_and_resets_for_next_request() {
        let (input, mut output) = tokio::io::duplex(64);
        output.write_all(b"1234567890").await.unwrap();
        let remaining = Arc::new(AtomicUsize::new(4));
        let mut bounded = Bounded {
            stream: input,
            remaining: remaining.clone(),
        };
        let mut bytes = [0; 16];
        assert_eq!(bounded.read(&mut bytes).await.unwrap(), 4);
        assert!(bounded.read(&mut bytes).await.is_err());
        remaining.store(6, Ordering::Relaxed);
        assert_eq!(bounded.read(&mut bytes).await.unwrap(), 6);
        assert_eq!(&bytes[..6], b"567890");
    }
}
