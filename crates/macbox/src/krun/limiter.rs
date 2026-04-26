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

#[cfg(test)]
mod tests {
    use super::*;
    use minibox_core::domain::ResourceLimiter;

    #[test]
    fn new_and_default_are_equivalent() {
        let _a = KrunLimiter::new();
        let _b = KrunLimiter::default();
    }

    #[test]
    fn create_returns_prefixed_group_id() {
        let limiter = KrunLimiter::new();
        let config = ResourceConfig::default();
        let group = limiter
            .create("test-ctr", &config)
            .expect("create should succeed");
        assert_eq!(group, "krun/test-ctr");
    }

    #[test]
    fn create_stores_config_retrievable_by_cleanup() {
        let limiter = KrunLimiter::new();
        let config = ResourceConfig {
            memory_limit_bytes: Some(512 * 1024 * 1024),
            cpu_weight: Some(100),
            ..Default::default()
        };
        limiter
            .create("ctr-1", &config)
            .expect("create should succeed");

        // Config is stored internally
        let map = limiter.configs.lock().expect("lock");
        assert!(map.contains_key("ctr-1"));
        assert_eq!(map["ctr-1"].memory_limit_bytes, Some(512 * 1024 * 1024));
        drop(map);

        // After cleanup, it's removed
        limiter.cleanup("ctr-1").expect("cleanup should succeed");
        let map = limiter.configs.lock().expect("lock");
        assert!(!map.contains_key("ctr-1"));
    }

    #[test]
    fn add_process_is_noop_ok() {
        let limiter = KrunLimiter::new();
        limiter
            .add_process("any-id", 12345)
            .expect("add_process should succeed");
    }

    #[test]
    fn cleanup_without_prior_create_is_ok() {
        let limiter = KrunLimiter::new();
        limiter
            .cleanup("never-created")
            .expect("cleanup should succeed even without prior create");
    }

    #[test]
    fn double_cleanup_is_ok() {
        let limiter = KrunLimiter::new();
        let config = ResourceConfig::default();
        limiter.create("ctr-2", &config).expect("create");
        limiter.cleanup("ctr-2").expect("first cleanup");
        limiter.cleanup("ctr-2").expect("second cleanup");
    }

    #[test]
    fn create_overwrites_previous_config() {
        let limiter = KrunLimiter::new();
        let config1 = ResourceConfig {
            memory_limit_bytes: Some(100),
            ..Default::default()
        };
        let config2 = ResourceConfig {
            memory_limit_bytes: Some(200),
            ..Default::default()
        };
        limiter.create("ctr-3", &config1).expect("first create");
        limiter.create("ctr-3", &config2).expect("second create");

        let map = limiter.configs.lock().expect("lock");
        assert_eq!(map["ctr-3"].memory_limit_bytes, Some(200));
    }

    #[test]
    fn as_any_downcasts_to_self() {
        use minibox_core::domain::AsAny;
        let limiter = KrunLimiter::new();
        assert!(limiter.as_any().downcast_ref::<KrunLimiter>().is_some());
    }
}
