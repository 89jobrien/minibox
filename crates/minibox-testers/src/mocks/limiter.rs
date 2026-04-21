//! Mock implementation of [`ResourceLimiter`].

use minibox_core::domain::{AsAny, ResourceConfig, ResourceLimiter};
use anyhow::Result;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// MockLimiter
// ---------------------------------------------------------------------------

/// Mock implementation of [`ResourceLimiter`] for testing.
///
/// Simulates cgroup operations without any kernel interaction. Returns a fake
/// cgroup path on success and tracks call counts.
#[derive(Debug, Clone)]
pub struct MockLimiter {
    state: Arc<Mutex<MockLimiterState>>,
}

#[derive(Debug)]
pub struct MockLimiterState {
    /// Whether `create` should succeed.
    create_should_succeed: bool,
    /// Whether `add_process` should succeed.
    add_process_should_succeed: bool,
    /// Whether `cleanup` should succeed.
    cleanup_should_succeed: bool,
    /// Running count of `create` invocations.
    create_count: usize,
    /// Running count of `cleanup` invocations.
    cleanup_count: usize,
    /// Container IDs for which a cgroup was successfully created.
    created_cgroups: Vec<String>,
}

impl MockLimiter {
    /// Create a new mock resource limiter with all operations succeeding by default.
    pub fn new() -> Self {
        Self {
            state: Arc::new(Mutex::new(MockLimiterState {
                create_should_succeed: true,
                add_process_should_succeed: true,
                cleanup_should_succeed: true,
                create_count: 0,
                cleanup_count: 0,
                created_cgroups: Vec::new(),
            })),
        }
    }

    /// Configure `create` to return an error.
    pub fn with_create_failure(self) -> Self {
        self.state.lock().unwrap().create_should_succeed = false;
        self
    }

    /// Return the number of times `create` has been called.
    pub fn create_count(&self) -> usize {
        self.state.lock().unwrap().create_count
    }

    /// Return the number of times `cleanup` has been called.
    pub fn cleanup_count(&self) -> usize {
        self.state.lock().unwrap().cleanup_count
    }
}

impl ResourceLimiter for MockLimiter {
    /// Simulate cgroup creation and return a fake cgroup path.
    ///
    /// Increments the create counter and records the container ID. Returns
    /// `/mock/cgroup/<container_id>` on success.
    fn create(&self, container_id: &str, _config: &ResourceConfig) -> Result<String> {
        let mut state = self.state.lock().unwrap();
        state.create_count += 1;

        if !state.create_should_succeed {
            anyhow::bail!("mock resource limiter create failure");
        }

        state.created_cgroups.push(container_id.to_string());
        Ok(format!("/mock/cgroup/{container_id}"))
    }

    /// Simulate adding a process to a cgroup — succeeds unless configured to fail.
    fn add_process(&self, _container_id: &str, _pid: u32) -> Result<()> {
        let state = self.state.lock().unwrap();
        if !state.add_process_should_succeed {
            anyhow::bail!("mock add_process failure");
        }
        Ok(())
    }

    /// Simulate cgroup cleanup.
    ///
    /// Increments the cleanup counter. Returns an error if the mock is
    /// configured to fail cleanup.
    fn cleanup(&self, _container_id: &str) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        state.cleanup_count += 1;

        if !state.cleanup_should_succeed {
            anyhow::bail!("mock cleanup failure");
        }
        Ok(())
    }
}

impl AsAny for MockLimiter {
    fn as_any(&self) -> &dyn ::std::any::Any {
        self
    }
}

impl Default for MockLimiter {
    fn default() -> Self {
        Self::new()
    }
}
