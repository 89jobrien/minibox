//! `KrunRuntime` — higher-level container lifecycle API backed by `SmolvmProcess`.
//!
//! Phase 1: delegates to `smolvm machine run` via [`SmolvmProcess`].
//! Phase 2: will replace subprocess calls with direct libkrun FFI.
//!
//! # API
//!
//! The krun lifecycle is:
//! 1. [`KrunRuntime::create`] — allocate a container ID and record config.
//! 2. [`KrunRuntime::start`] — spawn the microVM process.
//! 3. [`KrunRuntime::wait`] — wait for the process to exit, return exit code.
//! 4. [`KrunRuntime::stop`] — SIGTERM the process if still running.
//! 5. [`KrunRuntime::destroy`] — release all resources for the container.
//!
//! `create` + `start` are intentionally separate to mirror the
//! `ContainerRuntime` port's create/start lifecycle and to allow callers to
//! attach output readers between the two steps.

use crate::krun::process::SmolvmProcess;
use anyhow::{Context, Result, bail};
use minibox_core::domain::{
    AsAny, ContainerRuntime, RuntimeCapabilities, SpawnResult, ContainerSpawnConfig,
};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// State for a single container managed by `KrunRuntime`.
struct ContainerState {
    image: String,
    command: Vec<String>,
    env: Vec<(String, String)>,
    process: Option<SmolvmProcess>,
}

/// Higher-level container lifecycle manager backed by `SmolvmProcess`.
///
/// Maintains an in-memory registry of containers keyed by UUID.
/// Thread-safe via interior `Arc<Mutex<>>`.
pub struct KrunRuntime {
    containers: Arc<Mutex<HashMap<String, ContainerState>>>,
}

impl KrunRuntime {
    /// Create a new `KrunRuntime` with an empty container registry.
    pub fn new() -> Self {
        Self {
            containers: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Allocate a new container ID and record its configuration.
    ///
    /// Does **not** start the microVM. Call [`start`](Self::start) to boot it.
    ///
    /// # Returns
    ///
    /// A non-empty UUID string that identifies this container.
    pub async fn create(
        &self,
        image: &str,
        command: &[String],
        env: &[(String, String)],
    ) -> Result<String> {
        let id = Uuid::new_v4().to_string();
        let state = ContainerState {
            image: image.to_owned(),
            command: command.to_vec(),
            env: env.to_vec(),
            process: None,
        };
        self.containers.lock().await.insert(id.clone(), state);
        tracing::debug!(container_id = %id, image = %image, "krun: container created");
        Ok(id)
    }

    /// Start the microVM for a container previously created with [`create`](Self::create).
    ///
    /// # Errors
    ///
    /// Returns `Err` if the container ID is unknown, `smolvm` is not on PATH,
    /// or process spawn fails.
    pub async fn start(&self, id: &str) -> Result<()> {
        let mut map = self.containers.lock().await;
        let state = map
            .get_mut(id)
            .with_context(|| format!("krun: unknown container id {id}"))?;

        if state.process.is_some() {
            bail!("krun: container {id} is already started");
        }

        let proc = SmolvmProcess::spawn(&state.image, &state.command, &state.env)
            .await
            .with_context(|| format!("krun: spawn failed for container {id}"))?;

        state.process = Some(proc);
        tracing::info!(container_id = %id, image = %state.image, "krun: container started");
        Ok(())
    }

    /// Wait for the container process to exit and return its exit code.
    ///
    /// # Errors
    ///
    /// Returns `Err` if the container ID is unknown or the process has not
    /// been started.
    pub async fn wait(&self, id: &str) -> Result<i32> {
        let mut map = self.containers.lock().await;
        let state = map
            .get_mut(id)
            .with_context(|| format!("krun: unknown container id {id}"))?;

        let proc = state
            .process
            .as_mut()
            .with_context(|| format!("krun: container {id} has not been started"))?;

        let code = proc
            .wait()
            .await
            .with_context(|| format!("krun: wait failed for container {id}"))?;

        tracing::info!(container_id = %id, exit_code = code, "krun: container exited");
        Ok(code)
    }

    /// Send SIGTERM to the container process.
    ///
    /// No-op if the process is not running. Does not remove the container
    /// record — call [`destroy`](Self::destroy) after stop to release resources.
    pub async fn stop(&self, id: &str) -> Result<()> {
        let mut map = self.containers.lock().await;
        let state = map
            .get_mut(id)
            .with_context(|| format!("krun: unknown container id {id}"))?;

        if let Some(proc) = state.process.as_mut() {
            proc.stop()
                .await
                .with_context(|| format!("krun: stop failed for container {id}"))?;
            tracing::info!(container_id = %id, "krun: container stopped");
        }
        Ok(())
    }

    /// Remove the container record and release all associated resources.
    ///
    /// If the process is still running it is stopped first (best-effort).
    pub async fn destroy(&self, id: &str) -> Result<()> {
        let mut map = self.containers.lock().await;
        if let Some(mut state) = map.remove(id)
            && let Some(proc) = state.process.as_mut()
            && let Err(e) = proc.stop().await
        {
            tracing::warn!(
                container_id = %id,
                error = %e,
                "krun: best-effort stop on destroy failed"
            );
        }
        tracing::debug!(container_id = %id, "krun: container destroyed");
        Ok(())
    }

    /// Collect all stdout output from a running or exited container.
    ///
    /// Returns the output and implicitly waits for the process to finish.
    pub async fn collect_stdout(&self, id: &str) -> Result<String> {
        let mut map = self.containers.lock().await;
        let state = map
            .get_mut(id)
            .with_context(|| format!("krun: unknown container id {id}"))?;

        let proc = state
            .process
            .as_mut()
            .with_context(|| format!("krun: container {id} has not been started"))?;

        proc.collect_stdout()
            .await
            .with_context(|| format!("krun: collect_stdout failed for container {id}"))
    }
}

impl Default for KrunRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl AsAny for KrunRuntime {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

#[async_trait::async_trait]
impl ContainerRuntime for KrunRuntime {
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities {
            supports_user_namespaces: false,
            supports_cgroups_v2: false,
            supports_overlay_fs: false,
            supports_network_isolation: true,
            max_containers: None,
        }
    }

    /// Spawn a container via the krun/smolvm microVM backend.
    ///
    /// Uses the rootfs path as the image identifier passed to smolvm, and
    /// returns a synthetic PID (1) since krun manages its own process tree
    /// inside the VM rather than exposing a host-side PID.
    async fn spawn_process(&self, config: &ContainerSpawnConfig) -> Result<SpawnResult> {
        let env: Vec<(String, String)> = config
            .env
            .iter()
            .filter_map(|e| {
                let mut parts = e.splitn(2, '=');
                let k = parts.next()?.to_owned();
                let v = parts.next().unwrap_or("").to_owned();
                Some((k, v))
            })
            .collect();

        // Build the full command: [command] + args
        let mut command = vec![config.command.clone()];
        command.extend(config.args.clone());

        // Use the rootfs path as the smolvm image identifier.
        let image = config.rootfs.to_string_lossy().to_string();

        let id = self.create(&image, &command, &env).await?;
        self.start(&id).await?;

        Ok(SpawnResult {
            pid: 1,
            output_reader: None,
        })
    }
}
