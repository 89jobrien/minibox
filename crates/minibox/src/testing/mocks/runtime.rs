//! Mock implementation of [`ContainerRuntime`].

use anyhow::Result;
use async_trait::async_trait;
use minibox_core::domain::{
    AsAny, ContainerRuntime, ContainerSpawnConfig, RuntimeCapabilities, SpawnResult,
};
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// MockRuntime
// ---------------------------------------------------------------------------

/// Mock implementation of [`ContainerRuntime`] for testing.
///
/// Simulates container process spawning without any syscalls. Returns
/// monotonically increasing fake PIDs starting from 10000.
#[derive(Debug, Clone)]
pub struct MockRuntime {
    state: Arc<Mutex<MockRuntimeState>>,
}

#[derive(Debug)]
pub struct MockRuntimeState {
    /// Whether `spawn_process` calls should succeed.
    spawn_should_succeed: bool,
    /// The PID to hand out on the next successful spawn; incremented after each use.
    next_pid: u32,
    /// Running count of `spawn_process` invocations (both sync and async).
    spawn_count: usize,
}

impl MockRuntime {
    /// Create a new mock runtime with spawn succeeding and PIDs starting at 10000.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockRuntimeState {
                spawn_should_succeed: true,
                next_pid: 10000,
                spawn_count: 0,
            })),
        }
    }

    /// Configure all subsequent `spawn_process` calls to return an error.
    pub fn with_spawn_failure(self) -> Self {
        self.state.lock().unwrap().spawn_should_succeed = false;
        self
    }

    /// Return the total number of spawn attempts (successful and failed).
    pub fn spawn_count(&self) -> usize {
        self.state.lock().unwrap().spawn_count
    }

    /// Synchronous variant of `spawn_process` — bypasses async machinery.
    ///
    /// Useful in benchmarks and synchronous test helpers where an async
    /// executor is not available. Shares state with the async variant.
    pub fn spawn_process_sync(&self, _cfg: &ContainerSpawnConfig) -> Result<SpawnResult> {
        let mut state = self.state.lock().unwrap();
        state.spawn_count += 1;
        if !state.spawn_should_succeed {
            anyhow::bail!("mock spawn failure");
        }
        let pid = state.next_pid;
        state.next_pid += 1;
        Ok(SpawnResult {
            runtime_id: None,
            pid,
            output_reader: None,
        })
    }
}

#[async_trait]
impl ContainerRuntime for MockRuntime {
    /// Return minimal capabilities — the mock does not support any Linux-specific features.
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: false,
            supports_overlay_fs: false,
            supports_network_isolation: false,
            max_containers: None,
        }
    }

    /// Simulate spawning a container process and return a fake PID.
    ///
    /// Increments the spawn counter and the internal PID counter on success.
    /// The `output_reader` field is always `None`.
    async fn spawn_process(&self, _config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        let mut state = self.state.lock().unwrap();
        state.spawn_count += 1;

        if !state.spawn_should_succeed {
            anyhow::bail!("mock spawn failure");
        }

        let pid = state.next_pid;
        state.next_pid += 1;
        Ok(SpawnResult {
            runtime_id: None,
            pid,
            output_reader: None,
        })
    }
}

impl AsAny for MockRuntime {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

impl Default for MockRuntime {
    fn default() -> Self {
        Self::new()
    }
}
