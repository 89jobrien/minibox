//! `KrunLimiter` — resource limiter adapter for the krun/smolvm VM backend.
//!
//! krun sets resource limits at VM creation time via libkrun configuration
//! rather than through host-side cgroups.  This adapter stores the config
//! for Phase 3 wiring and returns `Ok` for all operations — it acts as a
//! no-op placeholder that validates the interface without side effects.

use anyhow::Result;
use minibox_core::domain::{ResourceConfig, ResourceLimiter};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Resource limiter adapter for the krun microVM backend.
///
/// Stores resource configs per container ID so Phase 3 can pass them to
/// libkrun at VM creation time.  All `ResourceLimiter` operations return
/// `Ok(())` — no cgroups are manipulated on the host.
pub struct KrunLimiter {
    configs: Arc<Mutex<HashMap<String, ResourceConfig>>>,
}

impl KrunLimiter {
    /// Create a new `KrunLimiter` with an empty config registry.
    pub fn new() -> Self {
        Self {
            configs: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl Default for KrunLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl minibox_core::domain::AsAny for KrunLimiter {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl ResourceLimiter for KrunLimiter {
    /// Record the resource config for `container_id`.
    ///
    /// Returns `Ok` and a placeholder group identifier; no cgroups are created.
    fn create(&self, container_id: &str, config: &ResourceConfig) -> Result<String> {
        self.configs
            .lock()
            .expect("krun limiter mutex poisoned")
            .insert(container_id.to_owned(), config.clone());

        tracing::debug!(
            container_id = %container_id,
            memory_limit_bytes = ?config.memory_limit_bytes,
            cpu_weight = ?config.cpu_weight,
            "krun: limiter config recorded"
        );

        Ok(format!("krun/{container_id}"))
    }

    /// No-op — krun does not use host cgroups for process membership.
    fn add_process(&self, _container_id: &str, _pid: u32) -> Result<()> {
        Ok(())
    }

    /// Remove the stored config, if any.  Always returns `Ok`.
    fn cleanup(&self, container_id: &str) -> Result<()> {
        self.configs
            .lock()
            .expect("krun limiter mutex poisoned")
            .remove(container_id);

        tracing::debug!(
            container_id = %container_id,
            "krun: limiter config removed"
        );

        Ok(())
    }
}
