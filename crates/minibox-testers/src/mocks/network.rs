//! Mock implementation of [`NetworkProvider`].

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::domain::{AsAny, NetworkConfig, NetworkProvider, NetworkStats};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// MockNetwork
// ---------------------------------------------------------------------------

/// Mock implementation of [`NetworkProvider`] for testing.
///
/// Simulates network setup and cleanup without any real syscalls or namespace
/// operations. Returns a fixed fake netns path on `setup` and tracks call
/// counts for `setup` and `cleanup`.
#[derive(Debug, Clone)]
pub struct MockNetwork {
    state: Arc<Mutex<MockNetworkState>>,
}

#[derive(Debug)]
pub struct MockNetworkState {
    /// Whether `setup` should succeed.
    setup_should_succeed: bool,
    /// Whether `cleanup` should succeed.
    cleanup_should_succeed: bool,
    /// Running count of `setup` invocations.
    setup_count: usize,
    /// Running count of `cleanup` invocations.
    cleanup_count: usize,
}

impl MockNetwork {
    /// Create a new mock network with all operations succeeding by default.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockNetworkState {
                setup_should_succeed: true,
                cleanup_should_succeed: true,
                setup_count: 0,
                cleanup_count: 0,
            })),
        }
    }

    /// Configure `setup` to return an error.
    pub fn with_setup_failure(self) -> Self {
        self.state.lock().unwrap().setup_should_succeed = false;
        self
    }

    /// Configure `cleanup` to return an error.
    pub fn with_cleanup_failure(self) -> Self {
        self.state.lock().unwrap().cleanup_should_succeed = false;
        self
    }

    /// Return the number of times `setup` has been called.
    pub fn setup_count(&self) -> usize {
        self.state.lock().unwrap().setup_count
    }

    /// Return the number of times `cleanup` has been called.
    pub fn cleanup_count(&self) -> usize {
        self.state.lock().unwrap().cleanup_count
    }
}

#[async_trait]
impl NetworkProvider for MockNetwork {
    /// Simulate network namespace setup and return a fixed fake netns path.
    ///
    /// Increments the setup counter. Returns an error if configured via
    /// [`with_setup_failure`].
    async fn setup(&self, _container_id: &str, _config: &NetworkConfig) -> Result<String> {
        let mut state = self.state.lock().unwrap();
        state.setup_count += 1;

        if !state.setup_should_succeed {
            anyhow::bail!("mock network setup failure");
        }

        Ok("/mock/netns".to_string())
    }

    /// Simulate attaching a container to its network namespace — always succeeds.
    async fn attach(&self, _container_id: &str, _pid: u32) -> Result<()> {
        Ok(())
    }

    /// Simulate network cleanup.
    ///
    /// Increments the cleanup counter. Returns an error if configured via
    /// [`with_cleanup_failure`].
    async fn cleanup(&self, _container_id: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.cleanup_count += 1;
        if !state.cleanup_should_succeed {
            anyhow::bail!("mock network cleanup failure");
        }
        Ok(())
    }

    /// Return default (all-zero) network statistics.
    async fn stats(&self, _container_id: &str) -> Result<NetworkStats> {
        Ok(NetworkStats::default())
    }
}

impl AsAny for MockNetwork {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

impl Default for MockNetwork {
    fn default() -> Self {
        Self::new()
    }
}
