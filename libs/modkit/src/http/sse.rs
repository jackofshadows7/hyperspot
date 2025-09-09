use axum::response::sse::{Event, KeepAlive, Sse};
use futures::{Stream, StreamExt};
use serde::Serialize;
use std::{convert::Infallible, time::Duration};
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;

/// Small typed SSE broadcaster built on `tokio::sync::broadcast`.
#[derive(Clone)]
pub struct SseBroadcaster<T> {
    tx: broadcast::Sender<T>,
}

impl<T: Clone + Send + 'static> SseBroadcaster<T> {
    /// Create broadcaster with bounded buffer size (events are dropped when lagging).
    pub fn new(capacity: usize) -> Self {
        let (tx, _rx) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Broadcast a single message to current subscribers.
    pub fn send(&self, value: T) {
        let _ = self.tx.send(value);
    }

    /// Subscribe to a typed stream of messages.
    pub fn subscribe_stream(&self) -> impl Stream<Item = T> {
        BroadcastStream::new(self.tx.subscribe()).filter_map(|res| async move { res.ok() })
    }

    /// Convert a typed stream into an SSE stream with JSON payloads.
    pub fn into_sse_stream<U>(stream: U) -> impl Stream<Item = Result<Event, Infallible>>
    where
        U: Stream<Item = T>,
        T: Serialize,
    {
        stream.map(|msg| {
            // Prefer JSON payload for the event's `data:`
            let ev = Event::default().json_data(&msg).unwrap_or_else(|_| {
                // Last resort: plain text to avoid breaking the stream on serialization errors.
                Event::default().data("serialization_error")
            });
            Ok(ev)
        })
    }

    /// Build an `axum::Sse` stream with keepalive pings.
    pub fn sse_response(&self) -> Sse<impl Stream<Item = Result<Event, Infallible>>>
    where
        T: Serialize,
    {
        let stream = Self::into_sse_stream(self.subscribe_stream());
        Sse::new(stream).keep_alive(
            KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("keepalive"),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{timeout, Duration};

    #[tokio::test]
    async fn broadcaster_delivers_single_event() {
        use futures::StreamExt;

        let b = SseBroadcaster::<u32>::new(16);
        let mut sub = Box::pin(b.subscribe_stream());
        b.send(42);
        let v = timeout(Duration::from_millis(200), sub.next())
            .await
            .unwrap();
        assert_eq!(v, Some(42));
    }
}
