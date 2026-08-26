//! Tokio sender adapter for domain `ProgressSink` in tests.

use async_trait::async_trait;
use minibox_core::domain::{DynProgressSink, ProgressClosed, ProgressSink};

#[derive(Debug)]
struct TokioSenderProgressSink<T> {
    sender: tokio::sync::mpsc::Sender<T>,
}

/// Wrap a Tokio sender as a dynamic domain progress sink.
pub fn tokio_progress_sink<T: Send + 'static>(
    sender: tokio::sync::mpsc::Sender<T>,
) -> DynProgressSink<T> {
    std::sync::Arc::new(TokioSenderProgressSink { sender })
}

#[async_trait]
impl<T: Send + 'static> ProgressSink<T> for TokioSenderProgressSink<T> {
    async fn send(&self, value: T) -> Result<(), ProgressClosed> {
        self.sender.send(value).await.map_err(|_| ProgressClosed)
    }
}
