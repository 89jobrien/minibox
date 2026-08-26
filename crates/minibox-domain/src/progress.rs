//! Runtime-neutral progress streaming port.

use async_trait::async_trait;
use std::sync::Arc;

/// Error returned when a progress receiver has closed.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("progress receiver is closed")]
pub struct ProgressClosed;

/// Async sink for streaming domain progress values.
#[async_trait]
pub trait ProgressSink<T: Send + 'static>: Send + Sync {
    /// Send one progress value.
    async fn send(&self, value: T) -> Result<(), ProgressClosed>;
}

#[async_trait]
impl<T: Send + 'static> ProgressSink<T> for Arc<dyn ProgressSink<T>> {
    async fn send(&self, value: T) -> Result<(), ProgressClosed> {
        (**self).send(value).await
    }
}

/// Shared type-erased progress sink.
pub type DynProgressSink<T> = Arc<dyn ProgressSink<T>>;
