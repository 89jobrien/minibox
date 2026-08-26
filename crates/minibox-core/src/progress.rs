//! Async-runtime adapters for domain progress sinks.

use async_trait::async_trait;
use minibox_domain::{DynProgressSink, ExecOutput, ExecOutputStream, ProgressClosed, ProgressSink};
use std::sync::Arc;

/// Adapts a Tokio MPSC sender to the runtime-neutral domain progress port.
#[derive(Debug)]
pub struct TokioProgressSink<T> {
    sender: tokio::sync::mpsc::Sender<T>,
}

impl<T> TokioProgressSink<T> {
    /// Create a sink around `sender`.
    #[must_use]
    pub const fn new(sender: tokio::sync::mpsc::Sender<T>) -> Self {
        Self { sender }
    }
}

impl<T: Send + 'static> TokioProgressSink<T> {
    /// Create a shared type-erased sink around `sender`.
    #[must_use]
    pub fn shared(sender: tokio::sync::mpsc::Sender<T>) -> DynProgressSink<T> {
        Arc::new(Self::new(sender))
    }
}

#[async_trait]
impl<T: Send + 'static> ProgressSink<T> for TokioProgressSink<T> {
    async fn send(&self, value: T) -> Result<(), ProgressClosed> {
        self.sender.send(value).await.map_err(|_| ProgressClosed)
    }
}

/// Bounded Tokio receiver exposed through the domain exec-output stream port.
pub struct TokioExecOutputStream {
    receiver: tokio::sync::mpsc::Receiver<ExecOutput>,
}

impl TokioExecOutputStream {
    /// Wrap an exec-output receiver.
    #[must_use]
    pub const fn new(receiver: tokio::sync::mpsc::Receiver<ExecOutput>) -> Self {
        Self { receiver }
    }

    /// Create an already-completed output stream.
    #[must_use]
    pub fn empty() -> Self {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(sender);
        Self { receiver }
    }
}

#[async_trait]
impl ExecOutputStream for TokioExecOutputStream {
    async fn next(&mut self) -> Option<ExecOutput> {
        self.receiver.recv().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn tokio_progress_sink_reports_closed_receiver() {
        let (sender, receiver) = tokio::sync::mpsc::channel(1);
        drop(receiver);
        let sink = TokioProgressSink::new(sender);

        assert_eq!(sink.send(1_u8).await, Err(ProgressClosed));
    }
}
