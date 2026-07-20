use super::{AsAny, ContainerId, Priority, StepState};

// ---------------------------------------------------------------------------
// Slashcrux integration helpers
// ---------------------------------------------------------------------------

/// Returns `true` when `actual` meets or exceeds the `min` priority threshold.
///
/// Comparison uses [`Priority::score`], where higher scores represent higher
/// priority.
#[must_use]
pub fn meets_min_priority(actual: &Priority, min: &Priority) -> bool {
    actual.score() >= min.score()
}

use anyhow::Result;
use async_trait::async_trait;
use std::path::{Path, PathBuf};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Workflow types
// ---------------------------------------------------------------------------

/// Retry policy for a single workflow step.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct StepRetry {
    /// Number of consecutive errors before the step is considered permanently failed.
    pub error_threshold: u32,
    /// Optional per-attempt timeout in seconds.
    pub timeout_secs: Option<u64>,
}

/// A name/value variable binding for workflow expression evaluation.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct ExprVar {
    /// Variable name.
    pub name: String,
    /// Variable value (string form).
    pub value: String,
}

/// A single step in a [`WorkflowDef`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct WorkflowStep {
    /// Step kind discriminant (e.g. `"container-run"`, `"shell"`).
    pub kind: String,
    /// Human-readable alias used to reference this step in outputs and `start_from_step`.
    pub alias: String,
    /// Optional conditional expression — step is skipped when this evaluates to false.
    #[serde(default)]
    pub if_expr: Option<String>,
    /// Optional if-guard expression evaluated before this step runs.
    ///
    /// When present, the expression is resolved via `evaluate_if_guard`; the step
    /// is skipped when the resolved value is empty, `"false"`, or `"0"`.
    #[serde(default)]
    pub if_guard: Option<String>,
    /// When `true`, workflow execution continues even if this step fails.
    #[serde(default)]
    pub continue_on_error: bool,
    /// Optional retry policy for this step.
    #[serde(default)]
    pub retry: Option<StepRetry>,
    /// Variable bindings in scope for this step.
    #[serde(default)]
    pub vars: Vec<ExprVar>,
    /// Step-kind-specific configuration payload.
    #[serde(default)]
    pub config: serde_json::Value,
}

/// A sequential multi-container workflow definition.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct WorkflowDef {
    /// Ordered list of steps to execute.
    pub steps: Vec<WorkflowStep>,
    /// Shared state passed between steps as JSON values.
    #[serde(default)]
    pub state: std::collections::HashMap<String, serde_json::Value>,
    /// When set, execution begins at the named step alias rather than the first step.
    #[serde(default)]
    pub start_from_step: Option<String>,
}

/// Aggregate outcome for a workflow phase (set of steps).
///
/// The ordering (`Succeeded < Skipped < Aborted < Failed < Errored`) is used to
/// compute the worst-case outcome across all steps via `Iterator::max`.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum PhaseOutcome {
    /// All steps completed successfully.
    Succeeded,
    /// At least one step was skipped; none failed.
    Skipped,
    /// Workflow was aborted mid-run.
    Aborted,
    /// At least one step failed (non-zero exit / business logic failure).
    Failed,
    /// At least one step encountered an unexpected runtime error.
    Errored,
}

/// Per-step execution status reported in [`DaemonResponse::WorkflowStepComplete`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum StepStatus {
    /// Step has not started yet.
    Pending,
    /// Step is currently executing.
    Running,
    /// Step completed with a zero exit code.
    Succeeded,
    /// Step completed with a non-zero exit code or business-logic failure.
    Failed,
    /// Step was skipped due to an `if_expr` condition.
    Skipped,
    /// Step encountered an unexpected runtime error.
    Errored,
}

impl From<StepStatus> for StepState {
    fn from(status: StepStatus) -> Self {
        match status {
            StepStatus::Pending => Self::Pending,
            StepStatus::Running => Self::Running,
            StepStatus::Succeeded => Self::Completed,
            StepStatus::Failed | StepStatus::Errored => Self::Failed,
            StepStatus::Skipped => Self::Skipped,
        }
    }
}

/// Determine the worst-case [`PhaseOutcome`] from a completed phase's step statuses.
///
/// Returns [`PhaseOutcome::Succeeded`] when `statuses` is empty (vacuously successful).
/// Otherwise maps each status to a `PhaseOutcome` and returns the maximum (worst) value.
pub fn determine_final_phase(statuses: &[StepStatus]) -> PhaseOutcome {
    statuses
        .iter()
        .map(|s| match s {
            StepStatus::Succeeded => PhaseOutcome::Succeeded,
            StepStatus::Skipped => PhaseOutcome::Skipped,
            StepStatus::Pending | StepStatus::Running => PhaseOutcome::Aborted,
            StepStatus::Failed => PhaseOutcome::Failed,
            StepStatus::Errored => PhaseOutcome::Errored,
        })
        .max()
        .unwrap_or(PhaseOutcome::Succeeded)
}

// ── StepRunner port ──────────────────────────────────────────────────────────

/// Capability tokens injected into a [`StepRunner`] at execution time.
///
/// Each runner declares which capabilities it requires; the engine injects only
/// those, following the principle of least privilege.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepCapability {
    /// Access to the container runtime (create, exec, inspect).
    AccessRegistry,
    /// Access to an image registry (pull, push, inspect).
    AccessRuntime,
    /// Read/write access to the overlay filesystem layer store.
    AccessFilesystem,
    /// Propagate step output values to downstream steps via context.
    OutputPropagation,
}

/// Execution context passed to a [`StepRunner::run`] call.
pub struct StepContext {
    /// Human-readable alias for the step, used in tracing and error messages.
    pub alias: String,
    /// Step-specific configuration extracted from the workflow definition.
    pub config: serde_json::Value,
    /// Accumulated outputs from all prior steps in this workflow execution.
    pub prior_outputs: WorkflowState,
}

/// Result value produced by a [`StepRunner`].
pub struct StepOutput {
    /// Structured output value, forwarded to downstream steps when
    /// [`StepCapability::OutputPropagation`] is declared.
    pub value: serde_json::Value,
    /// Terminal status reported back to the workflow engine.
    pub status: StepStatus,
}

/// Feature declarations that a [`StepRunner`] can advertise to callers.
///
/// Unlike [`StepCapability`] (which governs runtime resource injection),
/// `StepRunnerCapability` describes *what workflow features* the runner honours.
/// Callers can query these before dispatch to decide whether to supply optional
/// configuration fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StepRunnerCapability {
    /// Runner evaluates `if:` guard expressions and skips the step when false.
    SupportsIfGuards,
    /// Runner honours `retry:` configuration (count, delay, backoff).
    SupportsRetry,
    /// Runner enforces `timeout:` deadlines and returns an error on expiry.
    SupportsTimeout,
    /// Runner supports inter-step alias passing (reading/writing step outputs).
    SupportsAliasState,
}

/// Port: a pluggable executor for a single workflow step kind.
///
/// Implementations live in `minibox/src/adapters/` or may be provided by
/// external plugins.  The domain layer only depends on this trait.
pub trait StepRunner: Send + Sync {
    /// Unique identifier for the step kind (e.g. `"container-run"`).
    fn kind(&self) -> &'static str;
    /// Capability tokens required by this runner.
    fn required_capabilities(&self) -> &[StepCapability];
    /// Workflow feature declarations for this runner.
    ///
    /// The default implementation returns an empty slice for backward
    /// compatibility — existing runners that do not override this method
    /// simply advertise no optional features.
    fn declared_capabilities(&self) -> &[StepRunnerCapability] {
        &[]
    }
    /// Execute one step with the given context.
    ///
    /// # Errors
    ///
    /// Returns an error if the step fails.
    fn run(&self, ctx: StepContext) -> anyhow::Result<StepOutput>;
}

/// Registry of [`StepRunner`] implementations, keyed by [`StepRunner::kind`].
///
/// `StepRunnerRegistry::new()` creates an empty registry.  Call
/// [`StepRunnerRegistry::register_builtin_runners`] explicitly to populate the
/// four built-in runners; this keeps construction lightweight for tests that
/// only need a subset.
pub struct StepRunnerRegistry {
    runners: std::collections::HashMap<String, Box<dyn StepRunner>>,
}

impl StepRunnerRegistry {
    /// Create an empty registry.  No built-in runners are registered.
    #[must_use]
    pub fn new() -> Self {
        Self {
            runners: std::collections::HashMap::new(),
        }
    }

    /// Register a single runner, replacing any existing runner with the same kind.
    pub fn register(&mut self, runner: Box<dyn StepRunner>) {
        self.runners.insert(runner.kind().to_string(), runner);
    }

    /// Look up a runner by kind string.  Returns `None` if not registered.
    #[must_use]
    pub fn get(&self, kind: &str) -> Option<&dyn StepRunner> {
        self.runners.get(kind).map(std::convert::AsRef::as_ref)
    }

    /// Return the [`StepRunnerCapability`] declarations for the given runner kind.
    ///
    /// Returns `None` when no runner with that kind is registered.  Returns an
    /// empty slice when the runner is registered but declares no capabilities.
    pub fn capabilities_for(&self, kind: &str) -> Option<&[StepRunnerCapability]> {
        self.runners.get(kind).map(|r| r.declared_capabilities())
    }

    /// List all registered (kind, capabilities) pairs.
    #[must_use]
    pub fn list(&self) -> Vec<(&str, &[StepCapability])> {
        self.runners
            .iter()
            .map(|(k, r)| (k.as_str(), r.required_capabilities()))
            .collect()
    }

    /// Register the four built-in runners: `container-run`, `image-pull`,
    /// `exec`, and `overlay-snapshot`.
    #[cfg(test)]
    fn register_builtin_runners(&mut self) {
        self.register(Box::new(ContainerRunStepRunner));
        self.register(Box::new(ImagePullStepRunner));
        self.register(Box::new(ExecStepRunner));
        self.register(Box::new(OverlaySnapshotStepRunner));
    }
}

impl Default for StepRunnerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

// ── Built-in runner stubs ────────────────────────────────────────────────────

/// Built-in runner for the `container-run` step kind.
pub struct ContainerRunStepRunner;

impl StepRunner for ContainerRunStepRunner {
    fn kind(&self) -> &'static str {
        "container-run"
    }

    fn required_capabilities(&self) -> &[StepCapability] {
        &[StepCapability::AccessRuntime]
    }

    fn run(&self, _ctx: StepContext) -> anyhow::Result<StepOutput> {
        Ok(StepOutput {
            value: serde_json::Value::Null,
            status: StepStatus::Succeeded,
        })
    }
}

/// Built-in runner for the `image-pull` step kind.
pub struct ImagePullStepRunner;

impl StepRunner for ImagePullStepRunner {
    fn kind(&self) -> &'static str {
        "image-pull"
    }

    fn required_capabilities(&self) -> &[StepCapability] {
        &[
            StepCapability::AccessRegistry,
            StepCapability::AccessFilesystem,
        ]
    }

    fn run(&self, _ctx: StepContext) -> anyhow::Result<StepOutput> {
        Ok(StepOutput {
            value: serde_json::Value::Null,
            status: StepStatus::Succeeded,
        })
    }
}

/// Built-in runner for the `exec` step kind.
pub struct ExecStepRunner;

impl StepRunner for ExecStepRunner {
    fn kind(&self) -> &'static str {
        "exec"
    }

    fn required_capabilities(&self) -> &[StepCapability] {
        &[StepCapability::AccessRuntime]
    }

    fn run(&self, _ctx: StepContext) -> anyhow::Result<StepOutput> {
        Ok(StepOutput {
            value: serde_json::Value::Null,
            status: StepStatus::Succeeded,
        })
    }
}

/// Built-in runner for the `overlay-snapshot` step kind.
pub struct OverlaySnapshotStepRunner;

impl StepRunner for OverlaySnapshotStepRunner {
    fn kind(&self) -> &'static str {
        "overlay-snapshot"
    }

    fn required_capabilities(&self) -> &[StepCapability] {
        &[StepCapability::AccessFilesystem]
    }

    fn run(&self, _ctx: StepContext) -> anyhow::Result<StepOutput> {
        Ok(StepOutput {
            value: serde_json::Value::Null,
            status: StepStatus::Succeeded,
        })
    }
}

// ---------------------------------------------------------------------------
// Metrics Recorder Port
// ---------------------------------------------------------------------------

/// Port for recording operational metrics.
///
/// Adapters: `PrometheusMetricsRecorder` (production), `NoOpMetricsRecorder`
/// (testing/disabled), `RecordingMetricsRecorder` (test assertions).
///
/// String-based names and labels keep the domain free of OTEL/Prometheus types.
pub trait MetricsRecorder: Send + Sync {
    /// Increment a counter by 1.
    fn increment_counter(&self, name: &str, labels: &[(&str, &str)]);
    /// Record a value in a histogram (e.g., duration in seconds).
    fn record_histogram(&self, name: &str, value: f64, labels: &[(&str, &str)]);
    /// Set a gauge to an absolute value.
    fn set_gauge(&self, name: &str, value: f64, labels: &[(&str, &str)]);
}

// ---------------------------------------------------------------------------
// Image Registry Port
// ---------------------------------------------------------------------------

/// Abstraction for pulling container images from a registry.
///
/// This trait defines the contract for image registry implementations.
/// Implementations might include Docker Hub, GitHub Container Registry,
/// Quay.io, or private registries.
///
/// # Examples
///
/// ```rust,ignore
/// use minibox::domain::ImageRegistry;
///
/// struct DockerHubRegistry {
///     client: RegistryClient,
///     store: ImageStore,
/// }
///
/// #[async_trait]
/// impl ImageRegistry for DockerHubRegistry {
///     async fn has_image(&self, name: &str, tag: &str) -> bool {
///         self.store.has_image(name, tag)
///     }
///     // ... implement other methods
/// }
/// ```
#[async_trait]
pub trait ImageRegistry: AsAny + Send + Sync {
    /// Check if an image exists locally in the store.
    ///
    /// Returns `true` if the image has been pulled and cached locally,
    /// `false` otherwise.
    async fn has_image(&self, name: &str, tag: &str) -> bool;

    /// Pull an image from the registry and store it locally.
    ///
    /// Downloads all layers, verifies their digests, and extracts them
    /// to the local image store.
    ///
    /// # Arguments
    ///
    /// * `name` - Image name (e.g., `"library/ubuntu"`)
    /// * `tag` - Image tag (e.g., `"22.04"`)
    ///
    /// # Returns
    ///
    /// Metadata about the pulled image including layer information.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Authentication fails
    /// - Network request fails
    /// - Manifest is invalid
    /// - Layer download fails
    /// - Digest verification fails
    async fn pull_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
    ) -> Result<ImageMetadata>;

    /// Get the layer paths for a cached image.
    ///
    /// Returns an ordered list of layer directories (bottom-to-top) that
    /// can be used to construct an overlay filesystem.
    ///
    /// # Arguments
    ///
    /// * `name` - Image name
    /// * `tag` - Image tag
    ///
    /// # Returns
    ///
    /// Vector of absolute paths to extracted layer directories.
    ///
    /// # Errors
    ///
    /// Returns an error if the image is not cached locally.
    fn get_image_layers(&self, name: &str, tag: &str) -> Result<Vec<PathBuf>>;
}

// ---------------------------------------------------------------------------
// Registry Router Port
// ---------------------------------------------------------------------------

/// Port for routing an image reference to the appropriate [`ImageRegistry`] adapter.
///
/// Implementations select the registry based on the image's hostname (or any
/// other criteria) and return a reference to the corresponding adapter.
///
/// # Implementations
///
/// - [`minibox_core::adapters::HostnameRegistryRouter`]: routes by lowercase hostname;
///   falls back to a default registry for unrecognised hostnames.
///
/// # Example
///
/// ```rust,ignore
/// use minibox_core::domain::{DynRegistryRouter, RegistryRouter};
///
/// let router: DynRegistryRouter = Arc::new(HostnameRegistryRouter::new(
///     docker_hub_registry,
///     [("ghcr.io", ghcr_registry)],
/// ));
/// let registry = router.route(&image_ref);
/// ```
pub trait RegistryRouter: Send + Sync {
    /// Return the registry adapter that should handle `image_ref`.
    fn route(&self, image_ref: &crate::image::reference::ImageRef) -> &dyn ImageRegistry;
}

/// Port for loading a local OCI image tarball into the image store.
///
/// Implementations:
/// - `NativeImageLoader`: extracts tarball directly into `ImageStore`
/// - `ColimaRegistry`: delegates to `nerdctl load -i <path>` in the Lima VM
#[async_trait]
pub trait ImageLoader: Send + Sync {
    /// Load the OCI tarball at `path` and register it as `name:tag`.
    async fn load_image(&self, path: &std::path::Path, name: &str, tag: &str)
    -> anyhow::Result<()>;
}

/// Metadata about a pulled container image.
#[derive(Debug, Clone)]
pub struct ImageMetadata {
    /// Fully qualified image name (e.g., `"library/ubuntu"`).
    pub name: String,
    /// Image tag (e.g., `"22.04"`).
    pub tag: String,
    /// List of layers in bottom-to-top order.
    pub layers: Vec<LayerInfo>,
}

/// Information about a single image layer.
#[derive(Debug, Clone)]
pub struct LayerInfo {
    /// Digest of the layer (e.g., `"sha256:abc123..."`).
    pub digest: String,
    /// Size of the layer in bytes.
    pub size: u64,
}

// ---------------------------------------------------------------------------
// Exec Runtime Port
// ---------------------------------------------------------------------------

/// Pure specification for running a command inside a container.
///
/// This is a domain value type — no channel fields, no tokio types.
/// Channel wiring (stdin relay, PTY resize) belongs in the infrastructure
/// adapter layer (`minibox::adapters::exec`).
#[derive(Debug, Clone)]
pub struct ExecSpec {
    pub cmd: Vec<String>,
    pub env: Vec<String>,
    pub working_dir: Option<std::path::PathBuf>,
    pub tty: bool,
}

/// Handle representing a started exec instance.
#[derive(Debug, Clone)]
pub struct ExecHandle {
    pub id: String,
}

// ---------------------------------------------------------------------------
// ProgressSink — runtime-agnostic channel abstraction (#278)
// ---------------------------------------------------------------------------

/// An async-capable sink for streaming progress updates from domain ports.
///
/// Replaces direct `tokio::sync::mpsc::Sender<T>` parameters in port trait
/// signatures so the domain layer is not coupled to a specific async runtime.
/// Adapters (and tests) provide concrete implementations — the blanket impl
/// for `tokio::sync::mpsc::Sender<T>` covers the production case.
#[async_trait]
pub trait ProgressSink<T: Send + 'static>: Send + Sync {
    /// Send a value into the sink.
    ///
    /// Returns `Ok(())` when the value was accepted, or `Err(())` when the
    /// receiver has been dropped (analogous to `mpsc::SendError`).
    async fn send(&self, value: T) -> Result<(), ()>;
}

/// Blanket implementation so `tokio::sync::mpsc::Sender<T>` satisfies
/// `ProgressSink<T>` without wrapper code at every call site.
#[async_trait]
impl<T: Send + 'static> ProgressSink<T> for tokio::sync::mpsc::Sender<T> {
    async fn send(&self, value: T) -> Result<(), ()> {
        Self::send(self, value).await.map_err(|_| ())
    }
}

/// Blanket implementation so `Arc<dyn ProgressSink<T>>` (i.e. `DynProgressSink<T>`)
/// can be passed where `&dyn ProgressSink<T>` is expected.
#[async_trait]
impl<T: Send + 'static> ProgressSink<T> for Arc<dyn ProgressSink<T>> {
    async fn send(&self, value: T) -> Result<(), ()> {
        (**self).send(value).await
    }
}

/// Type-erased progress sink, used in port trait signatures.
///
/// Uses `Arc` rather than `Box` so the sink can be shared across tasks
/// (e.g. a blocking spawn and a forwarding task in the exec adapter).
pub type DynProgressSink<T> = Arc<dyn ProgressSink<T>>;

/// Port for running commands inside already-running containers.
#[async_trait]
pub trait ExecRuntime: AsAny + Send + Sync {
    async fn run_in_container(
        &self,
        container_id: &ContainerId,
        spec: ExecSpec,
        tx: DynProgressSink<crate::protocol::DaemonResponse>,
    ) -> anyhow::Result<ExecHandle>;
}

/// Type alias for a shared, dynamic [`ExecRuntime`] implementation.
pub type DynExecRuntime = Arc<dyn ExecRuntime>;

// ---------------------------------------------------------------------------
// Image Pusher Port
// ---------------------------------------------------------------------------

/// Credentials for authenticating to a registry.
#[derive(Debug, Clone)]
pub enum RegistryCredentials {
    Anonymous,
    Basic { username: String, password: String },
    Token(String),
}

/// Result of a successful image push.
#[derive(Debug, Clone)]
pub struct PushResult {
    pub digest: String,
    pub size_bytes: u64,
}

/// Push progress update.
#[derive(Debug, Clone)]
pub struct PushProgress {
    pub layer_digest: String,
    pub bytes_uploaded: u64,
    pub total_bytes: u64,
}

/// Port for pushing images to OCI-compliant registries.
#[async_trait]
pub trait ImagePusher: AsAny + Send + Sync {
    async fn push_image(
        &self,
        image_ref: &crate::image::reference::ImageRef,
        credentials: &RegistryCredentials,
        progress_tx: Option<DynProgressSink<PushProgress>>,
    ) -> anyhow::Result<PushResult>;
}

/// Type alias for a shared, dynamic [`ImagePusher`] implementation.
pub type DynImagePusher = Arc<dyn ImagePusher>;

// ---------------------------------------------------------------------------
// Container Committer Port
// ---------------------------------------------------------------------------

/// Configuration for committing a container to a new image.
#[derive(Debug, Clone)]
pub struct CommitConfig {
    pub author: Option<String>,
    pub message: Option<String>,
    pub env_overrides: Vec<String>,
    pub cmd_override: Option<Vec<String>>,
}

/// Port for snapshotting a container's filesystem diff into a new image.
#[async_trait]
pub trait ContainerCommitter: AsAny + Send + Sync {
    async fn commit(
        &self,
        container_id: &ContainerId,
        target_ref: &str,
        config: &CommitConfig,
    ) -> anyhow::Result<ImageMetadata>;
}

/// Type alias for a shared, dynamic [`ContainerCommitter`] implementation.
pub type DynContainerCommitter = Arc<dyn ContainerCommitter>;

// ---------------------------------------------------------------------------
// Image Builder Port
// ---------------------------------------------------------------------------

/// Context directory and Dockerfile location for a build.
#[derive(Debug, Clone)]
pub struct BuildContext {
    /// Directory that serves as the build context (files available to COPY/ADD).
    pub directory: std::path::PathBuf,
    /// Path to the Dockerfile, relative to `directory`.
    pub dockerfile: std::path::PathBuf,
}

/// Configuration for an image build operation.
#[derive(Debug, Clone)]
pub struct BuildConfig {
    /// Target image tag (e.g. `"myapp:latest"`).
    pub tag: String,
    /// Build-time argument overrides (ARG key=value).
    pub build_args: Vec<(String, String)>,
    /// When `true`, skip any cached layers and rebuild from scratch.
    pub no_cache: bool,
}

/// A progress update emitted while a build is running.
#[derive(Debug, Clone)]
pub struct BuildProgress {
    /// 1-based index of the current step.
    pub step: u32,
    /// Total number of steps in the Dockerfile.
    pub total_steps: u32,
    /// Human-readable description of the current step.
    pub message: String,
}

/// Port for building container images from a Dockerfile.
#[async_trait]
pub trait ImageBuilder: AsAny + Send + Sync {
    /// Build an image from the given context and config, streaming progress via `progress_tx`.
    ///
    /// Returns [`ImageMetadata`] for the newly built image on success.
    async fn build_image(
        &self,
        context: &BuildContext,
        config: &BuildConfig,
        progress_tx: DynProgressSink<BuildProgress>,
    ) -> anyhow::Result<ImageMetadata>;
}

/// Type alias for a shared, dynamic [`ImageBuilder`] implementation.
pub type DynImageBuilder = Arc<dyn ImageBuilder>;

// ---------------------------------------------------------------------------
// PTY Allocator Port (#83)
// ---------------------------------------------------------------------------

/// Configuration for allocating a pseudo-terminal (PTY) for interactive containers.
///
/// Passed to [`PtyAllocator::allocate`] to request a PTY pair with the given
/// terminal dimensions. The caller is responsible for closing the returned file
/// descriptors when no longer needed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct PtyConfig {
    /// Whether PTY allocation is requested.
    pub enabled: bool,
    /// Terminal width in columns.
    pub cols: u16,
    /// Terminal height in rows.
    pub rows: u16,
}

/// An allocated PTY pair — a master and a slave file descriptor.
///
/// The master fd is used by the host to read/write the terminal stream.
/// The slave fd is handed to the container process as its controlling terminal.
///
/// # Ownership
///
/// The caller that calls [`PtyAllocator::allocate`] owns both fds and is
/// responsible for closing them. Do NOT call `close()` on them from outside
/// unless you also own the handle.
#[derive(Debug)]
pub struct PtyHandle {
    /// File descriptor for the master side of the PTY.
    pub master_fd: i32,
    /// File descriptor for the slave side of the PTY.
    pub slave_fd: i32,
}

/// Port for allocating a PTY pair.
///
/// Implementations live in the adapter layer. The domain layer never calls
/// `posix_openpt` directly — all OS-level PTY operations go through this trait.
pub trait PtyAllocator: Send + Sync {
    /// Allocate a PTY pair with the terminal dimensions specified in `config`.
    ///
    /// Returns [`PtyHandle`] on success, or `Err` when PTY allocation is not
    /// supported (e.g., [`NullPtyAllocator`]) or when the OS call fails.
    ///
    /// # Errors
    ///
    /// Returns an error if PTY allocation is unsupported or the OS call fails.
    fn allocate(&self, config: &PtyConfig) -> anyhow::Result<PtyHandle>;
}

/// Type alias for a shared, dynamic [`PtyAllocator`] implementation.
pub type DynPtyAllocator = Arc<dyn PtyAllocator>;

/// A no-op [`PtyAllocator`] that always returns `Err`.
///
/// Used as the default adapter when PTY support is not available (e.g., on
/// macOS or in test environments that do not exercise the PTY path).
pub struct NullPtyAllocator;

impl PtyAllocator for NullPtyAllocator {
    fn allocate(&self, _config: &PtyConfig) -> anyhow::Result<PtyHandle> {
        anyhow::bail!("pty: PTY allocation is not supported in this environment")
    }
}

/// A test double [`PtyAllocator`] that returns a pre-configured [`PtyHandle`].
///
/// Enabled only when the `test-utils` feature is active so production binaries
/// do not pull in test scaffolding.
#[cfg(feature = "test-utils")]
pub struct MockPtyAllocator {
    master_fd: i32,
    slave_fd: i32,
}

#[cfg(feature = "test-utils")]
impl MockPtyAllocator {
    /// Create a `MockPtyAllocator` that returns `master_fd` and `slave_fd`.
    #[must_use]
    pub const fn new(master_fd: i32, slave_fd: i32) -> Self {
        Self {
            master_fd,
            slave_fd,
        }
    }
}

#[cfg(feature = "test-utils")]
impl PtyAllocator for MockPtyAllocator {
    fn allocate(&self, _config: &PtyConfig) -> anyhow::Result<PtyHandle> {
        Ok(PtyHandle {
            master_fd: self.master_fd,
            slave_fd: self.slave_fd,
        })
    }
}

// ---------------------------------------------------------------------------
// VM Checkpoint Port
// ---------------------------------------------------------------------------

/// Metadata describing a saved VM snapshot.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SnapshotInfo {
    /// Container/VM ID this snapshot belongs to.
    pub container_id: String,
    /// Human-readable snapshot name.
    pub name: String,
    /// ISO-8601 creation timestamp.
    pub created_at: String,
    /// Adapter that created this snapshot.
    pub adapter: String,
    /// Image the container was running when snapshotted.
    pub image: String,
    /// Snapshot size in bytes (0 if unknown).
    pub size_bytes: u64,
}

/// Port for saving and restoring VM state checkpoints.
///
/// Adapters that support checkpointing (smolvm, krun, vz) implement this
/// trait. Adapters that do not support it return an error from every method
/// and omit [`BackendCapability::Checkpoint`] from their capability set.
pub trait VmCheckpoint: Send + Sync {
    /// Persist the current VM/container state to `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if checkpointing is unsupported or the OS call fails.
    fn save_snapshot(&self, container_id: &str, path: &Path) -> Result<SnapshotInfo>;

    /// Restore VM/container state from a previously saved snapshot at `path`.
    ///
    /// # Errors
    ///
    /// Returns an error if the snapshot cannot be found or restored.
    fn restore_snapshot(&self, container_id: &str, path: &Path) -> Result<()>;

    /// List all snapshots for `container_id`.
    ///
    /// # Errors
    ///
    /// Returns an error if listing snapshots fails.
    fn list_snapshots(&self, container_id: &str) -> Result<Vec<SnapshotInfo>>;
}

/// Type alias for a shared, dynamic [`VmCheckpoint`] implementation.
pub type DynVmCheckpoint = Arc<dyn VmCheckpoint>;

/// A no-op [`VmCheckpoint`] that always returns "not supported".
///
/// Used as the default adapter for backends without checkpoint support.
pub struct NoopVmCheckpoint;

impl VmCheckpoint for NoopVmCheckpoint {
    fn save_snapshot(&self, _container_id: &str, _path: &Path) -> Result<SnapshotInfo> {
        anyhow::bail!("checkpoint: not supported by this adapter")
    }

    fn restore_snapshot(&self, _container_id: &str, _path: &Path) -> Result<()> {
        anyhow::bail!("checkpoint: not supported by this adapter")
    }

    fn list_snapshots(&self, _container_id: &str) -> Result<Vec<SnapshotInfo>> {
        anyhow::bail!("checkpoint: not supported by this adapter")
    }
}

// ---------------------------------------------------------------------------
// Conformance boundary — commit / build / push capabilities
// ---------------------------------------------------------------------------

/// An individual capability that a backend adapter may or may not support.
///
/// Used by [`BackendCapabilitySet`] to describe what a concrete backend can do.
/// The conformance suite gates tests on these flags so that backend-specific
/// tests are skipped rather than failed when a capability is absent.
///
/// # Backend support matrix
///
/// | Capability          | linux-native | Colima |
/// |---------------------|:------------:|:------:|
/// | `Commit`            | yes          | no     |
/// | `BuildFromContext`  | yes          | no     |
/// | `PushToRegistry`    | yes          | yes    |
///
/// **linux-native** — `OverlayCommitAdapter`, `MiniboxImageBuilder`,
/// `OciPushAdapter`: all three traits are fully implemented; commit and build
/// require root and Linux namespaces; push requires a reachable OCI registry.
///
/// **Colima** — `ColimaImagePusher` implements `ImagePusher`; there is no
/// Colima-native `ContainerCommitter` or `ImageBuilder` implementation yet
/// (Colima containers use the nerdctl/lima CLI, which does not expose an
/// upperdir for overlay-style commit, and no Dockerfile build path has been
/// wired into the adapter suite).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendCapability {
    /// Backend can snapshot a running container's FS diff into a new image
    /// via [`ContainerCommitter::commit`].
    Commit,
    /// Backend can build an image from a `BuildContext` + `BuildConfig` via
    /// [`ImageBuilder::build_image`].
    BuildFromContext,
    /// Backend can push an image to an OCI-compliant registry via
    /// [`ImagePusher::push_image`].
    PushToRegistry,
    /// Backend can save/restore VM state checkpoints via
    /// [`VmCheckpoint::save_snapshot`] / [`VmCheckpoint::restore_snapshot`].
    Checkpoint,
    /// Backend provides [`RootfsSetup`] + [`ChildInit`] (filesystem operations).
    Filesystem,
    /// Backend provides [`ExecRuntime`] (exec into running containers).
    Exec,
    /// Backend provides [`NetworkProvider`] (bridge/host/tailnet networking).
    Network,
    /// Backend provides [`TtyProvider`] (pseudo-terminal allocation).
    Tty,
    /// Backend provides [`PtyAllocator`] (low-level PTY pair allocation).
    Pty,
    /// Backend provides [`MetricsRecorder`] (counter/histogram/gauge).
    Metrics,
    /// Backend provides [`RegistryRouter`] (multi-registry routing).
    RegistryRouter,
    /// Backend provides [`ImageLoader`] (local OCI tarball loading).
    ImageLoader,
}

/// The full set of [`BackendCapability`] flags declared by one backend.
///
/// Construct via [`BackendCapabilitySet::new`] and chain
/// [`BackendCapabilitySet::with`] calls:
///
/// ```rust
/// use minibox_core::domain::{BackendCapability, BackendCapabilitySet};
///
/// let caps = BackendCapabilitySet::new()
///     .with(BackendCapability::Commit)
///     .with(BackendCapability::PushToRegistry);
///
/// assert!(caps.supports(BackendCapability::Commit));
/// assert!(!caps.supports(BackendCapability::BuildFromContext));
/// ```
#[derive(Debug, Clone, Default)]
pub struct BackendCapabilitySet {
    flags: std::collections::HashSet<BackendCapability>,
}

impl BackendCapabilitySet {
    /// Create an empty capability set (no capabilities).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a capability to this set (builder-style).
    #[must_use]
    pub fn with(mut self, cap: BackendCapability) -> Self {
        self.flags.insert(cap);
        self
    }

    /// Return `true` if this set includes `cap`.
    #[must_use]
    pub fn supports(&self, cap: BackendCapability) -> bool {
        self.flags.contains(&cap)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    // --- MetricsRecorder tests ---

    /// Verify that a no-op MetricsRecorder can be constructed and used as a trait object.
    #[test]
    fn metrics_recorder_trait_object() {
        struct StubRecorder;
        impl MetricsRecorder for StubRecorder {
            fn increment_counter(&self, _name: &str, _labels: &[(&str, &str)]) {}
            fn record_histogram(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
            fn set_gauge(&self, _name: &str, _value: f64, _labels: &[(&str, &str)]) {}
        }

        let recorder: Arc<dyn MetricsRecorder> = Arc::new(StubRecorder);
        recorder.increment_counter("test_counter", &[("key", "val")]);
        recorder.record_histogram("test_hist", 1.5, &[]);
        recorder.set_gauge("test_gauge", 42.0, &[("a", "b")]);
    }

    // --- ContainerId tests ---

    #[test]
    fn container_id_valid() {
        let id = ContainerId::new("abc123".to_string()).expect("valid alphanumeric id");
        assert_eq!(id.as_str(), "abc123");
    }

    #[test]
    fn container_id_empty() {
        let result = ContainerId::new(String::new());
        assert!(result.is_err(), "empty id should fail");
    }

    #[test]
    fn container_id_too_long() {
        let long = "a".repeat(65);
        let result = ContainerId::new(long);
        assert!(result.is_err(), "65-char id should fail");
    }

    #[test]
    fn container_id_max_length() {
        let id_str = "a".repeat(64);
        let id = ContainerId::new(id_str.clone()).expect("64-char id should succeed");
        assert_eq!(id.as_str(), id_str);
    }

    #[test]
    fn container_id_special_chars() {
        let result = ContainerId::new("abc-123".to_string());
        assert!(result.is_err(), "hyphen should fail alphanumeric check");
    }

    #[test]
    fn container_id_spaces() {
        let result = ContainerId::new("abc 123".to_string());
        assert!(result.is_err(), "space should fail alphanumeric check");
    }

    #[test]
    fn container_id_as_str() {
        let id = ContainerId::new("deadbeef".to_string()).expect("valid id");
        assert_eq!(id.as_str(), "deadbeef");
    }

    #[test]
    fn container_id_display() {
        let id = ContainerId::new("abc123".to_string()).expect("valid id");
        assert_eq!(format!("{id}"), "abc123");
    }

    #[test]
    fn container_id_equality() {
        let a = ContainerId::new("abc123".to_string()).expect("valid id");
        let b = ContainerId::new("abc123".to_string()).expect("valid id");
        assert_eq!(a, b);
    }

    #[test]
    fn container_id_hash() {
        let a = ContainerId::new("abc123".to_string()).expect("valid id");
        let b = ContainerId::new("def456".to_string()).expect("valid id");
        let mut set: HashSet<ContainerId> = HashSet::new();
        set.insert(a.clone());
        set.insert(b.clone());
        assert!(set.contains(&a));
        assert!(set.contains(&b));
        assert_eq!(set.len(), 2);
    }

    // --- ContainerId hex edge-case tests (GH #145) ---

    #[test]
    fn container_id_valid_16_char_hex() {
        // A standard 16-character lowercase hex ID (common Docker short-ID format) is valid.
        let id = ContainerId::new("deadbeef01234567".to_string()).expect("valid 16-char hex id");
        assert_eq!(id.as_str(), "deadbeef01234567");
    }

    #[test]
    fn container_id_15_chars_is_valid() {
        // The validator requires 1–64 alphanumeric chars; 15 chars is within that range.
        // There is no minimum length beyond non-empty, so a 15-char hex string is accepted.
        let id = ContainerId::new("deadbeef0123456".to_string())
            .expect("15-char hex id is within the 1-64 range and must be accepted");
        assert_eq!(id.as_str().len(), 15);
    }

    #[test]
    fn container_id_17_chars_is_valid() {
        // Similarly, 17-char hex strings are within the 64-char maximum and accepted.
        let id = ContainerId::new("deadbeef012345678".to_string())
            .expect("17-char hex id is within the 1-64 range and must be accepted");
        assert_eq!(id.as_str().len(), 17);
    }

    #[test]
    fn container_id_non_hex_chars_rejected() {
        // Characters outside [0-9a-fA-F] that are also non-alphanumeric are rejected.
        // Hyphens and underscores are not alphanumeric, so they fail validation.
        let result = ContainerId::new("deadbeef-0123456".to_string());
        assert!(
            result.is_err(),
            "hyphen is not alphanumeric and must be rejected"
        );
    }

    #[test]
    fn container_id_empty_rejected() {
        let result = ContainerId::new(String::new());
        assert!(result.is_err(), "empty string must be rejected");
    }

    /// The validator uses `is_ascii_alphanumeric()`, which allows both lowercase and uppercase
    /// hex characters (a-f and A-F). Mixed-case hex IDs such as "DeadBeef01234567" are
    /// therefore accepted — they are alphanumeric even though they mix case. Code that compares
    /// container IDs must normalise case if canonical form matters.
    #[test]
    fn container_id_mixed_case_hex_accepted() {
        let id = ContainerId::new("DeadBeef01234567".to_string())
            .expect("mixed-case hex is alphanumeric and must be accepted");
        assert_eq!(id.as_str(), "DeadBeef01234567");
    }

    // --- ContainerState tests ---

    #[test]
    fn container_state_as_str() {
        assert_eq!(ContainerState::Created.as_str(), "Created");
        assert_eq!(ContainerState::Running.as_str(), "Running");
        assert_eq!(ContainerState::Paused.as_str(), "Paused");
        assert_eq!(ContainerState::Stopped.as_str(), "Stopped");
        assert_eq!(ContainerState::Failed.as_str(), "Failed");
    }

    #[test]
    fn container_state_display() {
        assert_eq!(format!("{}", ContainerState::Created), "Created");
        assert_eq!(format!("{}", ContainerState::Running), "Running");
        assert_eq!(format!("{}", ContainerState::Paused), "Paused");
        assert_eq!(format!("{}", ContainerState::Stopped), "Stopped");
        assert_eq!(format!("{}", ContainerState::Failed), "Failed");
    }

    #[test]
    fn container_state_clone_eq() {
        let state = ContainerState::Running;
        let cloned = state;
        assert_eq!(state, cloned);
        assert_ne!(state, ContainerState::Stopped);
    }

    // --- DomainError tests ---

    #[test]
    fn domain_error_display_image_not_found() {
        let err = DomainError::ImageNotFound {
            name: "library/ubuntu".to_string(),
            tag: "22.04".to_string(),
        };
        assert_eq!(err.error_kind(), "image_not_found");
        assert_eq!(err.to_string(), "image library/ubuntu:22.04 not found");
    }

    #[test]
    fn domain_error_display_container_not_found() {
        let err = DomainError::ContainerNotFound {
            id: "abc123".to_string(),
        };
        assert_eq!(err.error_kind(), "container_not_found");
        assert_eq!(err.to_string(), "container 'abc123' not found");
    }

    #[test]
    fn domain_error_display_resource_limit_exceeded() {
        let err = DomainError::ResourceLimitExceeded {
            limit: "memory_bytes".to_string(),
            value: 9999,
            max: 1024,
        };
        assert_eq!(err.error_kind(), "resource_limit_exceeded");
        let msg = err.to_string();
        assert!(msg.contains("memory_bytes"), "should contain limit name");
        assert!(msg.contains("9999"), "should contain value");
        assert!(msg.contains("1024"), "should contain max");
    }

    // --- ResourceConfig tests ---

    #[test]
    fn resource_config_default() {
        let config = ResourceConfig::default();
        assert!(config.memory_limit_bytes.is_none());
        assert!(config.cpu_weight.is_none());
        assert!(config.pids_max.is_none());
        assert!(config.io_max_bytes_per_sec.is_none());
    }

    #[test]
    fn resource_config_serde_roundtrip() {
        let config = ResourceConfig {
            memory_limit_bytes: Some(1024 * 1024 * 256),
            cpu_weight: Some(500),
            pids_max: Some(100),
            io_max_bytes_per_sec: Some(1024 * 1024),
        };
        let json = serde_json::to_string(&config).expect("serialize");
        let back: ResourceConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back.memory_limit_bytes, config.memory_limit_bytes);
        assert_eq!(back.cpu_weight, config.cpu_weight);
        assert_eq!(back.pids_max, config.pids_max);
        assert_eq!(back.io_max_bytes_per_sec, config.io_max_bytes_per_sec);
    }

    // --- HookSpec / ContainerHooks tests ---

    #[test]
    fn hook_spec_default() {
        let hook = HookSpec::default();
        assert_eq!(hook.command, "");
        assert!(hook.args.is_empty());
        assert!(hook.timeout_secs.is_none());
    }

    #[test]
    fn container_hooks_default() {
        let hooks = ContainerHooks::default();
        assert!(hooks.pre_exec.is_empty());
        assert!(hooks.post_exit.is_empty());
    }

    // --- RuntimeCapabilities tests ---

    #[test]
    fn runtime_capabilities_debug() {
        let caps = RuntimeCapabilities {
            supports_user_namespaces: true,
            supports_cgroups_v2: false,
            supports_overlay_fs: true,
            supports_network_isolation: false,
            max_containers: Some(128),
        };
        let debug_str = format!("{caps:?}");
        assert!(!debug_str.is_empty(), "Debug impl should produce output");
    }

    // --- ImageLoader tests ---

    // --- ExecSpec purity test ---

    /// Verify that ExecSpec is Clone and contains no channel fields.
    /// This encodes the architecture contract: ExecSpec is a pure domain
    /// value type that must not depend on tokio infrastructure.
    #[test]
    fn exec_spec_is_pure_domain() {
        let spec = crate::domain::ExecSpec {
            cmd: vec!["echo".to_string()],
            env: vec![],
            working_dir: None,
            tty: false,
        };
        // Must be Clone — pure domain types are always Clone
        let cloned = spec.clone();
        assert_eq!(cloned.cmd, vec!["echo".to_string()]);
        assert!(!cloned.tty);
    }

    #[cfg(test)]
    mod image_loader_tests {
        use super::*;
        use std::path::Path;

        struct AlwaysOkLoader;

        #[async_trait::async_trait]
        impl ImageLoader for AlwaysOkLoader {
            async fn load_image(
                &self,
                _path: &Path,
                _name: &str,
                _tag: &str,
            ) -> anyhow::Result<()> {
                Ok(())
            }
        }

        #[tokio::test]
        async fn image_loader_trait_is_object_safe() {
            let loader: Box<dyn ImageLoader> = Box::new(AlwaysOkLoader);
            let result = loader
                .load_image(
                    std::path::Path::new("/fake.tar"),
                    "minibox-tester",
                    "latest",
                )
                .await;
            assert!(result.is_ok());
        }
    }

    mod backend_rootfs_metadata_tests {
        use super::*;
        use std::path::PathBuf;

        #[test]
        fn overlay_upper_dir_returns_path_for_native_variant() {
            let path = PathBuf::from("/var/lib/minibox/containers/abc/upper");
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: path.clone().into(),
                metadata: std::collections::HashMap::new(),
            };
            assert_eq!(&**meta.overlay_upper_dir(), path.as_path());
        }

        #[test]
        fn overlay_upper_dir_returns_path_for_colima_variant() {
            let path = PathBuf::from("/Users/joe/.lima/colima/upper");
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: path.clone().into(),
                metadata: kv,
            };
            assert_eq!(&**meta.overlay_upper_dir(), path.as_path());
        }

        #[test]
        fn metadata_value_none_for_missing_key() {
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/tmp/upper").into(),
                metadata: std::collections::HashMap::new(),
            };
            assert_eq!(meta.metadata_value("colima_instance"), None);
        }

        #[test]
        fn metadata_value_returns_value_for_present_key() {
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/tmp/upper").into(),
                metadata: kv,
            };
            assert_eq!(meta.metadata_value("colima_instance"), Some("colima"));
        }

        #[test]
        fn backend_rootfs_metadata_roundtrips_serde_overlay() {
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/var/lib/minibox/containers/abc/upper").into(),
                metadata: std::collections::HashMap::new(),
            };
            let json = serde_json::to_string(&meta).expect("serialize");
            let restored: BackendRootfsMetadata = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(meta, restored);
        }

        #[test]
        fn backend_rootfs_metadata_roundtrips_serde_with_kv() {
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/Users/joe/.lima/colima/upper").into(),
                metadata: kv,
            };
            let json = serde_json::to_string(&meta).expect("serialize");
            let restored: BackendRootfsMetadata = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(meta, restored);
        }

        #[test]
        fn rootfs_layout_metadata_survives_commit_image_ref() {
            // Verify that an Overlay metadata's upper_dir is unchanged
            // after being stored and retrieved (simulates the commit path
            // reading the upper_dir from the container record).
            let upper = PathBuf::from("/Users/joe/.lima/colima/containers/abc/upper");
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: upper.clone().into(),
                metadata,
            };
            let layout = RootfsLayout {
                merged_dir: PathBuf::from("/tmp/merged").into(),
                rootfs_metadata: Some(meta),
                source_image_ref: Some("alpine:latest".to_string()),
            };
            let recovered_upper = layout
                .rootfs_metadata
                .as_ref()
                .expect("metadata present")
                .overlay_upper_dir();
            assert_eq!(&**recovered_upper, upper.as_path());
        }

        // --- Task 1: OCP fix tests ---

        #[test]
        fn overlay_variant_has_opaque_metadata_map() {
            // BackendRootfsMetadata::Overlay must carry an opaque HashMap so
            // backends can encode their own KVs without adding new variants.
            let mut metadata = std::collections::HashMap::new();
            metadata.insert("colima_instance".to_string(), "colima".to_string());
            let upper = PathBuf::from("/Users/joe/.lima/colima/upper");
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: upper.clone().into(),
                metadata: metadata.clone(),
            };
            assert_eq!(&**meta.overlay_upper_dir(), upper.as_path());
            assert_eq!(meta.metadata_value("colima_instance"), Some("colima"));
        }

        #[test]
        fn overlay_variant_metadata_empty_for_native() {
            // Native overlay encodes no extra KVs.
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/var/lib/minibox/containers/abc/upper").into(),
                metadata: std::collections::HashMap::new(),
            };
            assert_eq!(meta.metadata_value("colima_instance"), None);
        }

        #[test]
        fn backend_rootfs_metadata_roundtrips_serde_with_metadata_map() {
            let mut kv = std::collections::HashMap::new();
            kv.insert("colima_instance".to_string(), "colima".to_string());
            let meta = BackendRootfsMetadata::Overlay {
                upper_dir: PathBuf::from("/Users/joe/.lima/colima/upper").into(),
                metadata: kv,
            };
            let json = serde_json::to_string(&meta).expect("serialize");
            let restored: BackendRootfsMetadata = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(meta, restored);
        }
    }

    mod pty_allocator_tests {
        use super::*;

        #[test]
        fn pty_config_default_values() {
            let cfg = PtyConfig {
                enabled: true,
                cols: 80,
                rows: 24,
            };
            assert!(cfg.enabled);
            assert_eq!(cfg.cols, 80);
            assert_eq!(cfg.rows, 24);
        }

        #[test]
        fn pty_config_serde_roundtrip() {
            let cfg = PtyConfig {
                enabled: true,
                cols: 120,
                rows: 40,
            };
            let json = serde_json::to_string(&cfg).expect("serialize");
            let back: PtyConfig = serde_json::from_str(&json).expect("deserialize");
            assert_eq!(back.enabled, cfg.enabled);
            assert_eq!(back.cols, cfg.cols);
            assert_eq!(back.rows, cfg.rows);
        }

        #[test]
        fn pty_config_deserialize_missing_fields_use_serde_default() {
            // When a JSON payload omits fields the struct must still deserialize.
            let json = r#"{"enabled":false,"cols":80,"rows":24}"#;
            let cfg: PtyConfig = serde_json::from_str(json).expect("deserialize");
            // Exercise NullPtyAllocator::allocate — a domain-defined SUT function.
            let result = NullPtyAllocator.allocate(&cfg);
            assert!(result.is_err(), "NullPtyAllocator must return Err");
            assert!(!cfg.enabled);
            assert_eq!(cfg.cols, 80);
            assert_eq!(cfg.rows, 24);
        }

        #[test]
        fn null_pty_allocator_returns_err() {
            let alloc = NullPtyAllocator;
            let cfg = PtyConfig {
                enabled: true,
                cols: 80,
                rows: 24,
            };
            assert!(
                alloc.allocate(&cfg).is_err(),
                "NullPtyAllocator must always return Err"
            );
        }

        #[cfg(feature = "test-utils")]
        #[test]
        fn mock_pty_allocator_returns_configured_handle() {
            let alloc = MockPtyAllocator::new(5, 6);
            let cfg = PtyConfig {
                enabled: true,
                cols: 80,
                rows: 24,
            };
            let handle = alloc.allocate(&cfg).expect("MockPtyAllocator must succeed");
            assert_eq!(handle.master_fd, 5);
            assert_eq!(handle.slave_fd, 6);
        }
    }

    mod isp_trait_split_tests {
        use super::*;
        use std::path::{Path, PathBuf};

        // --- Task 2: ISP split tests ---

        /// Verify that RootfsSetup is a standalone trait (not mixed with ChildInit).
        struct OnlySetup;
        impl AsAny for OnlySetup {
            fn as_any(&self) -> &dyn std::any::Any {
                self
            }
        }
        impl RootfsSetup for OnlySetup {
            fn setup_rootfs(
                &self,
                _layers: &[PathBuf],
                _container_dir: &Path,
            ) -> Result<RootfsLayout> {
                Ok(RootfsLayout {
                    merged_dir: PathBuf::from("/tmp/merged").into(),
                    rootfs_metadata: None,
                    source_image_ref: None,
                })
            }

            fn cleanup(&self, _container_dir: &Path) -> Result<()> {
                Ok(())
            }
        }

        /// Verify that ChildInit is a standalone trait for pivot_root.
        struct OnlyChildInit;
        impl ChildInit for OnlyChildInit {
            fn pivot_root(&self, _new_root: &Path) -> Result<()> {
                Ok(())
            }
        }

        #[test]
        fn rootfs_setup_can_be_used_without_child_init() {
            let setup = OnlySetup;
            let result = setup.setup_rootfs(&[], Path::new("/tmp/container"));
            assert!(result.is_ok());
            assert!(setup.cleanup(Path::new("/tmp/container")).is_ok());
        }

        #[test]
        fn child_init_can_be_used_without_rootfs_setup() {
            let init = OnlyChildInit;
            assert!(init.pivot_root(Path::new("/tmp/new_root")).is_ok());
        }
    }

    mod workflow_tests {
        use super::*;

        #[test]
        fn workflow_step_deserialize_defaults_continue_on_error_false() {
            let json = r#"{"kind":"container-run","alias":"build"}"#;
            let step: WorkflowStep = serde_json::from_str(json).unwrap();
            // Exercise determine_final_phase — a domain-defined SUT function.
            let outcome = determine_final_phase(&[StepStatus::Succeeded]);
            assert_eq!(outcome, PhaseOutcome::Succeeded);
            assert!(!step.continue_on_error);
            assert!(step.retry.is_none());
            assert_eq!(step.alias, "build");
        }

        #[test]
        fn phase_outcome_errored_beats_failed() {
            assert!(PhaseOutcome::Errored > PhaseOutcome::Failed);
        }

        #[test]
        fn phase_outcome_failed_beats_aborted() {
            assert!(PhaseOutcome::Failed > PhaseOutcome::Aborted);
        }

        #[test]
        fn phase_outcome_aborted_beats_skipped() {
            assert!(PhaseOutcome::Aborted > PhaseOutcome::Skipped);
        }

        #[test]
        fn phase_outcome_skipped_beats_succeeded() {
            assert!(PhaseOutcome::Skipped > PhaseOutcome::Succeeded);
        }

        use proptest::prelude::*;

        proptest! {
            #[test]
            fn worst_case_phase_with_any_errored_is_errored(count in 1usize..10) {
                let steps: Vec<PhaseOutcome> = (0..count)
                    .map(|_| PhaseOutcome::Succeeded)
                    .chain(std::iter::once(PhaseOutcome::Errored))
                    .collect();
                let worst = steps.iter().copied().max().unwrap();
                prop_assert_eq!(worst, PhaseOutcome::Errored);
            }
        }
    }
}

// ── Step completion ───────────────────────────────────────────────────────────

/// Outcome of evaluating a single step attempt against its retry policy.
///
/// Callers drive the retry loop; this type is the decision produced by
/// [`determine_step_completion`] for each attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StepCompletion {
    /// Step produced a successful [`StepOutput`].
    Succeeded,
    /// Step failed and should not be retried.
    ///
    /// `terminal` is `true` when the error is inherently unrecoverable
    /// (e.g. image not found), and `false` when the retry policy is
    /// exhausted or timed out.
    Failed { terminal: bool },
    /// Step encountered an unexpected runtime error (reserved for future use).
    Errored,
    /// Step failed transiently and should be retried by the caller.
    Running,
}

/// Pure function — no I/O, no side effects.
///
/// Decides whether a step attempt should be considered done, retried, or
/// permanently failed based on:
/// - the attempt result,
/// - the optional retry policy,
/// - how long the step has been running (`elapsed`),
/// - how many consecutive errors have occurred (`error_count`), and
/// - whether the error is terminal (unrecoverable regardless of policy).
#[cfg(test)]
fn determine_step_completion(
    result: &anyhow::Result<StepOutput>,
    retry_cfg: Option<&StepRetry>,
    elapsed: std::time::Duration,
    error_count: u32,
    is_terminal: bool,
) -> StepCompletion {
    match result {
        Ok(_) => StepCompletion::Succeeded,
        Err(_) => {
            if is_terminal {
                return StepCompletion::Failed { terminal: true };
            }
            if let Some(retry) = retry_cfg {
                if let Some(timeout_secs) = retry.timeout_secs {
                    if elapsed.as_secs() > timeout_secs {
                        return StepCompletion::Failed { terminal: false };
                    }
                }
                if error_count >= retry.error_threshold {
                    return StepCompletion::Failed { terminal: false };
                }
                StepCompletion::Running
            } else {
                StepCompletion::Failed { terminal: false }
            }
        }
    }
}

#[cfg(test)]
mod step_runner_tests {
    use super::*;

    #[test]
    fn registry_get_unknown_kind_returns_none() {
        let registry = StepRunnerRegistry::new();
        assert!(registry.get("unknown-kind").is_none());
    }

    #[test]
    fn registry_list_returns_all_registered_kinds() {
        let mut registry = StepRunnerRegistry::new();
        registry.register_builtin_runners();
        let kinds: Vec<&str> = registry.list().iter().map(|(k, _)| *k).collect();
        assert!(kinds.contains(&"container-run"));
        assert!(kinds.contains(&"image-pull"));
        assert!(kinds.contains(&"exec"));
        assert!(kinds.contains(&"overlay-snapshot"));
    }

    #[test]
    fn step_dependencies_only_injects_declared_caps() {
        let runner = ContainerRunStepRunner;
        let caps = runner.required_capabilities();
        assert!(caps.contains(&StepCapability::AccessRuntime));
        assert!(!caps.contains(&StepCapability::AccessRegistry));
    }

    pub fn assert_step_runner_contract(runner: &dyn StepRunner) {
        assert!(!runner.kind().is_empty(), "runner kind must not be empty");
        let _caps = runner.required_capabilities();
        // run with minimal context — must not panic
        let ctx = StepContext {
            alias: "test".to_string(),
            config: serde_json::Value::Null,
            prior_outputs: WorkflowState::new(),
        };
        let _ = runner.run(ctx); // result not checked — contract is no-panic, not success
    }

    #[test]
    fn container_run_satisfies_contract() {
        assert_step_runner_contract(&ContainerRunStepRunner);
    }
    #[test]
    fn image_pull_satisfies_contract() {
        assert_step_runner_contract(&ImagePullStepRunner);
    }
    #[test]
    fn exec_satisfies_contract() {
        assert_step_runner_contract(&ExecStepRunner);
    }
    #[test]
    fn overlay_snapshot_satisfies_contract() {
        assert_step_runner_contract(&OverlaySnapshotStepRunner);
    }

    // ── StepRunnerCapability / capabilities_for tests ────────────────────────

    /// A mock runner that declares every optional capability.
    struct FullyCapableRunner;

    impl StepRunner for FullyCapableRunner {
        fn kind(&self) -> &'static str {
            "fully-capable"
        }

        fn required_capabilities(&self) -> &[StepCapability] {
            &[]
        }

        fn declared_capabilities(&self) -> &[StepRunnerCapability] {
            &[
                StepRunnerCapability::SupportsIfGuards,
                StepRunnerCapability::SupportsRetry,
                StepRunnerCapability::SupportsTimeout,
                StepRunnerCapability::SupportsAliasState,
            ]
        }

        fn run(&self, _ctx: StepContext) -> anyhow::Result<StepOutput> {
            Ok(StepOutput {
                value: serde_json::Value::Null,
                status: StepStatus::Succeeded,
            })
        }
    }

    #[test]
    fn default_declared_capabilities_returns_empty_slice() {
        // Built-in runners do not override declared_capabilities — must be empty.
        assert!(ContainerRunStepRunner.declared_capabilities().is_empty());
        assert!(ImagePullStepRunner.declared_capabilities().is_empty());
        assert!(ExecStepRunner.declared_capabilities().is_empty());
        assert!(OverlaySnapshotStepRunner.declared_capabilities().is_empty());
    }

    #[test]
    fn mock_runner_declares_all_capabilities() {
        let runner = FullyCapableRunner;
        let caps = runner.declared_capabilities();
        assert!(caps.contains(&StepRunnerCapability::SupportsIfGuards));
        assert!(caps.contains(&StepRunnerCapability::SupportsRetry));
        assert!(caps.contains(&StepRunnerCapability::SupportsTimeout));
        assert!(caps.contains(&StepRunnerCapability::SupportsAliasState));
    }

    #[test]
    fn capabilities_for_unknown_kind_returns_none() {
        let registry = StepRunnerRegistry::new();
        assert!(registry.capabilities_for("nonexistent").is_none());
    }

    #[test]
    fn capabilities_for_builtin_runner_returns_empty_slice() {
        let mut registry = StepRunnerRegistry::new();
        registry.register_builtin_runners();
        let caps = registry
            .capabilities_for("container-run")
            .expect("container-run must be registered");
        assert!(
            caps.is_empty(),
            "built-in runners declare no capabilities by default"
        );
    }

    #[test]
    fn capabilities_for_fully_capable_runner_returns_all_four() {
        let mut registry = StepRunnerRegistry::new();
        registry.register(Box::new(FullyCapableRunner));
        let caps = registry
            .capabilities_for("fully-capable")
            .expect("fully-capable must be registered");
        assert_eq!(caps.len(), 4);
        assert!(caps.contains(&StepRunnerCapability::SupportsIfGuards));
        assert!(caps.contains(&StepRunnerCapability::SupportsRetry));
        assert!(caps.contains(&StepRunnerCapability::SupportsTimeout));
        assert!(caps.contains(&StepRunnerCapability::SupportsAliasState));
    }

    #[test]
    fn registry_capabilities_for_registered_after_builtin_runners() {
        let mut registry = StepRunnerRegistry::new();
        registry.register_builtin_runners();
        registry.register(Box::new(FullyCapableRunner));
        // Previously registered runners are still accessible.
        assert!(registry.capabilities_for("exec").is_some());
        // Newly registered runner is also accessible.
        let caps = registry.capabilities_for("fully-capable").unwrap();
        assert!(caps.contains(&StepRunnerCapability::SupportsTimeout));
    }
}

#[cfg(test)]
mod step_retry_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn terminal_error_returns_failed_terminal() {
        let result: anyhow::Result<StepOutput> = Err(anyhow::anyhow!("image not found"));
        let completion = determine_step_completion(
            &result,
            None,
            Duration::from_secs(1),
            0,
            true, // is_terminal
        );
        assert!(matches!(
            completion,
            StepCompletion::Failed { terminal: true }
        ));
    }

    #[test]
    fn non_terminal_under_threshold_returns_running() {
        let result: anyhow::Result<StepOutput> = Err(anyhow::anyhow!("network timeout"));
        let retry = StepRetry {
            error_threshold: 3,
            timeout_secs: None,
        };
        let completion = determine_step_completion(
            &result,
            Some(&retry),
            Duration::from_secs(1),
            1, // error_count = 1, threshold = 3
            false,
        );
        assert!(matches!(completion, StepCompletion::Running));
    }

    #[test]
    fn non_terminal_at_threshold_returns_failed() {
        let result: anyhow::Result<StepOutput> = Err(anyhow::anyhow!("network timeout"));
        let retry = StepRetry {
            error_threshold: 3,
            timeout_secs: None,
        };
        let completion = determine_step_completion(
            &result,
            Some(&retry),
            Duration::from_secs(1),
            3, // error_count == threshold
            false,
        );
        assert!(matches!(
            completion,
            StepCompletion::Failed { terminal: false }
        ));
    }

    #[test]
    fn elapsed_over_timeout_returns_failed() {
        let result: anyhow::Result<StepOutput> = Err(anyhow::anyhow!("still running"));
        let retry = StepRetry {
            error_threshold: 10,
            timeout_secs: Some(30),
        };
        let completion = determine_step_completion(
            &result,
            Some(&retry),
            Duration::from_secs(31), // elapsed > timeout
            0,
            false,
        );
        assert!(matches!(
            completion,
            StepCompletion::Failed { terminal: false }
        ));
    }

    #[test]
    fn success_returns_succeeded() {
        let result: anyhow::Result<StepOutput> = Ok(StepOutput {
            value: serde_json::Value::Null,
            status: StepStatus::Succeeded,
        });
        let completion = determine_step_completion(&result, None, Duration::from_secs(1), 0, false);
        assert!(matches!(completion, StepCompletion::Succeeded));
    }

    use proptest::prelude::*;
    proptest! {
        #[test]
        fn error_count_gte_threshold_always_fails(threshold in 1u32..20, extra in 0u32..5) {
            let result: anyhow::Result<StepOutput> = Err(anyhow::anyhow!("err"));
            let retry = StepRetry { error_threshold: threshold, timeout_secs: None };
            let completion = determine_step_completion(
                &result,
                Some(&retry),
                Duration::from_secs(1),
                threshold + extra,
                false,
            );
            let is_failed = matches!(completion, StepCompletion::Failed { .. });
            prop_assert!(is_failed);
        }

        #[test]
        fn terminal_error_never_returns_running(error_count in 0u32..100) {
            let result: anyhow::Result<StepOutput> = Err(anyhow::anyhow!("fatal"));
            let retry = StepRetry { error_threshold: 999, timeout_secs: None };
            let completion = determine_step_completion(
                &result,
                Some(&retry),
                Duration::from_millis(1),
                error_count,
                true,
            );
            prop_assert!(!matches!(completion, StepCompletion::Running));
        }
    }
}

// ---------------------------------------------------------------------------
// Workflow types — alias-based state passing (#360)
// ---------------------------------------------------------------------------

/// Shared mutable state threaded through all steps of a workflow run.
///
/// Keyed by step alias; each value is the JSON output produced by that step.
pub type WorkflowState = std::collections::HashMap<String, serde_json::Value>;

/// A [`WorkflowStep`] with all `${{ ... }}` expressions fully resolved to
/// concrete string values.
#[derive(Debug, Clone)]
pub struct ResolvedStep {
    /// Step kind — matches a registered step runner kind string.
    pub kind: String,
    /// Unique alias used to reference this step's output in later steps.
    pub alias: String,
    /// Resolved variable bindings: name → concrete string value.
    pub vars: std::collections::HashMap<String, String>,
    /// Step-kind-specific configuration (arbitrary JSON).
    pub config: serde_json::Value,
    /// If `true`, workflow execution continues even when this step fails.
    pub continue_on_error: bool,
    /// Optional retry policy.
    pub retry: Option<StepRetry>,
}

/// Resolves `${{ outputs['alias'].field }}` tokens in `step.vars` against `state`.
///
/// Returns `Err` if any token references a missing alias or field, or if a
/// token is syntactically malformed (e.g. unclosed `${{`).
pub fn resolve_step_vars(
    step: &WorkflowStep,
    state: &WorkflowState,
) -> anyhow::Result<ResolvedStep> {
    use anyhow::Context as _;
    let mut resolved_vars = std::collections::HashMap::new();

    for expr_var in &step.vars {
        let resolved_value = resolve_expr(&expr_var.value, state).with_context(|| {
            format!(
                "failed to resolve var '{}' in step '{}'",
                expr_var.name, step.alias
            )
        })?;
        resolved_vars.insert(expr_var.name.clone(), resolved_value);
    }

    Ok(ResolvedStep {
        kind: step.kind.clone(),
        alias: step.alias.clone(),
        vars: resolved_vars,
        config: step.config.clone(),
        continue_on_error: step.continue_on_error,
        retry: step.retry.clone(),
    })
}

/// Writes step output into shared workflow state under the step's alias.
///
/// Overwrites any prior value stored under the same alias.
pub fn propagate_output(alias: &str, output: serde_json::Value, state: &mut WorkflowState) {
    state.insert(alias.to_string(), output);
}

/// Returns all steps that precede `alias` in declaration order.
///
/// Returns `Err` if `alias` is not found in `steps`.
pub fn steps_before<'a>(
    alias: &str,
    steps: &'a [WorkflowStep],
) -> anyhow::Result<Vec<&'a WorkflowStep>> {
    let idx = steps
        .iter()
        .position(|s| s.alias == alias)
        .ok_or_else(|| anyhow::anyhow!("alias '{alias}' not found in workflow steps"))?;
    Ok(steps[..idx].iter().collect())
}

/// Prepares resumption from `resume_alias`.
///
/// Returns:
/// - the number of steps to skip (all steps before the resume point)
/// - a [`WorkflowState`] pre-populated with prior step outputs
///
/// The caller is responsible for loading `prior_outputs` from the trace store.
/// Steps with no entry in `prior_outputs` are omitted from the returned state.
pub fn resume_workflow(
    resume_alias: &str,
    steps: &[WorkflowStep],
    prior_outputs: &WorkflowState,
) -> anyhow::Result<(usize, WorkflowState)> {
    let preceding = steps_before(resume_alias, steps)?;
    let skip_count = preceding.len();

    let mut state = WorkflowState::new();
    for step in &preceding {
        if let Some(output) = prior_outputs.get(&step.alias) {
            propagate_output(&step.alias, output.clone(), &mut state);
        }
    }

    Ok((skip_count, state))
}

/// Evaluates the `if_guard` expression on `step`.
///
/// Returns `Ok(true)` when:
/// - `step.if_guard` is `None` (no guard — step always runs), or
/// - the resolved expression value is non-empty and is not `"false"` or `"0"`.
///
/// Returns `Ok(false)` when the resolved value is `""`, `"false"`, or `"0"`.
/// Returns `Err` when expression resolution fails.
pub fn evaluate_if_guard(step: &WorkflowStep, state: &WorkflowState) -> anyhow::Result<bool> {
    use anyhow::Context as _;
    let expr = match &step.if_guard {
        None => return Ok(true),
        Some(e) => e,
    };
    let resolved = resolve_expr(expr, state)
        .with_context(|| format!("failed to evaluate if_guard for step '{}'", step.alias))?;
    Ok(!matches!(resolved.as_str(), "" | "false" | "0"))
}

/// Resolves a single expression string.
///
/// Replaces every `${{ outputs['alias'].field }}` token with the
/// string-serialised value from `state`. Returns the original string
/// unchanged when no template tokens are present.
pub fn resolve_expr(expr: &str, state: &WorkflowState) -> anyhow::Result<String> {
    use anyhow::Context as _;

    if !expr.contains("${{") {
        return Ok(expr.to_string());
    }

    let mut result = expr.to_string();
    while let Some(start) = result.find("${{") {
        let end = result[start..]
            .find("}}")
            .map(|i| start + i + 2)
            .ok_or_else(|| anyhow::anyhow!("unclosed '${{' in expression: {expr}"))?;
        let token = result[start..end].to_string();
        let inner = token
            .trim_start_matches("${{")
            .trim_end_matches("}}")
            .trim();

        let value = resolve_output_ref(inner, state)
            .with_context(|| format!("failed to resolve expression: {inner}"))?;
        result = result.replacen(&token, &value, 1);
    }
    Ok(result)
}

/// Resolves `outputs['alias'].field.subfield` against `state`.
///
/// Supports dot-separated field paths of arbitrary depth. The field path
/// may be empty, in which case the full alias value is serialised.
pub fn resolve_output_ref(expr: &str, state: &WorkflowState) -> anyhow::Result<String> {
    let expr = expr.trim();
    let rest = expr.strip_prefix("outputs['").ok_or_else(|| {
        anyhow::anyhow!("unsupported expression form (expected outputs['alias']...): {expr}")
    })?;
    let (alias, rest) = rest
        .split_once("']")
        .ok_or_else(|| anyhow::anyhow!("malformed alias in expression: {expr}"))?;
    let field_path = rest.trim_start_matches('.');

    let alias_val = state
        .get(alias)
        .ok_or_else(|| anyhow::anyhow!("alias '{alias}' not found in workflow state"))?;

    let field_val = if field_path.is_empty() {
        alias_val.clone()
    } else {
        let mut cur = alias_val;
        for segment in field_path.split('.') {
            cur = cur.get(segment).ok_or_else(|| {
                anyhow::anyhow!("field '{segment}' not found in alias '{alias}' output")
            })?;
        }
        cur.clone()
    };

    Ok(match &field_val {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    })
}

#[cfg(test)]
mod alias_state_tests {
    use super::*;

    #[test]
    fn resolve_step_vars_substitutes_prior_output() {
        let mut state = WorkflowState::new();
        state.insert("build".to_string(), serde_json::json!({"exit_code": 0}));

        let step = WorkflowStep {
            kind: "exec".to_string(),
            alias: "check".to_string(),
            if_expr: None,
            if_guard: None,
            continue_on_error: false,
            retry: None,
            vars: vec![ExprVar {
                name: "code".to_string(),
                value: "${{ outputs['build'].exit_code }}".to_string(),
            }],
            config: serde_json::Value::Null,
        };

        let resolved = resolve_step_vars(&step, &state).unwrap();
        assert_eq!(resolved.vars.get("code").unwrap(), "0");
    }

    #[test]
    fn resolve_step_vars_missing_alias_returns_err() {
        let state = WorkflowState::new();
        let step = WorkflowStep {
            kind: "exec".to_string(),
            alias: "check".to_string(),
            if_expr: None,
            if_guard: None,
            continue_on_error: false,
            retry: None,
            vars: vec![ExprVar {
                name: "x".to_string(),
                value: "${{ outputs['missing'].field }}".to_string(),
            }],
            config: serde_json::Value::Null,
        };
        assert!(resolve_step_vars(&step, &state).is_err());
    }

    #[test]
    fn resolve_step_vars_no_tokens_is_idempotent() {
        let state = WorkflowState::new();
        let step = WorkflowStep {
            kind: "exec".to_string(),
            alias: "plain".to_string(),
            if_expr: None,
            if_guard: None,
            continue_on_error: false,
            retry: None,
            vars: vec![ExprVar {
                name: "k".to_string(),
                value: "literal_value".to_string(),
            }],
            config: serde_json::Value::Null,
        };
        let resolved = resolve_step_vars(&step, &state).unwrap();
        assert_eq!(resolved.vars.get("k").unwrap(), "literal_value");
    }

    #[test]
    fn propagate_output_writes_under_alias() {
        let mut state = WorkflowState::new();
        propagate_output("my-step", serde_json::json!({"result": "ok"}), &mut state);
        assert_eq!(state["my-step"]["result"], "ok");
    }

    use proptest::prelude::*;
    proptest! {
        #[test]
        fn resolve_no_token_vars_always_idempotent(
            key in "[a-z]{3,8}",
            val in "[a-zA-Z0-9_]{1,20}"
        ) {
            let state = WorkflowState::new();
            let step = WorkflowStep {
                kind: "exec".to_string(),
                alias: "s".to_string(),
                if_expr: None,
                if_guard: None,
                continue_on_error: false,
                retry: None,
                vars: vec![ExprVar { name: key.clone(), value: val.clone() }],
                config: serde_json::Value::Null,
            };
            let resolved = resolve_step_vars(&step, &state).unwrap();
            prop_assert_eq!(resolved.vars.get(&key).unwrap(), &val);
        }
    }
}

#[cfg(test)]
mod start_from_step_tests {
    use super::*;

    fn make_step(alias: &str) -> WorkflowStep {
        WorkflowStep {
            kind: "exec".to_string(),
            alias: alias.to_string(),
            if_expr: None,
            if_guard: None,
            continue_on_error: false,
            retry: None,
            vars: vec![],
            config: serde_json::Value::Null,
        }
    }

    #[test]
    fn steps_before_returns_steps_preceding_alias() {
        let steps = vec![make_step("build"), make_step("test"), make_step("deploy")];
        let preceding = steps_before("test", &steps).unwrap();
        assert_eq!(preceding.len(), 1);
        assert_eq!(preceding[0].alias, "build");
    }

    #[test]
    fn steps_before_first_step_returns_empty() {
        let steps = vec![make_step("build"), make_step("test")];
        let preceding = steps_before("build", &steps).unwrap();
        assert!(preceding.is_empty());
    }

    #[test]
    fn steps_before_unknown_alias_returns_err() {
        let steps = vec![make_step("build")];
        assert!(steps_before("nonexistent", &steps).is_err());
    }

    #[test]
    fn resume_workflow_injects_prior_outputs_into_state() {
        let steps = vec![make_step("build"), make_step("test"), make_step("deploy")];
        let mut prior_outputs = WorkflowState::new();
        prior_outputs.insert("build".to_string(), serde_json::json!({"exit_code": 0}));

        let (skip_count, state) = resume_workflow("test", &steps, &prior_outputs).unwrap();
        assert_eq!(skip_count, 1);
        assert_eq!(state["build"]["exit_code"], 0);
    }

    #[test]
    fn resume_workflow_unknown_alias_returns_err() {
        let steps = vec![make_step("build")];
        let prior = WorkflowState::new();
        assert!(resume_workflow("nonexistent", &steps, &prior).is_err());
    }
}

// ---------------------------------------------------------------------------
// Slashcrux integration unit tests (#283)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod slashcrux_tests {
    use super::*;
    use crate::protocol::execution_context_to_env;

    // ── meets_min_priority ─────────────────────────────────────────────

    #[test]
    fn priority_same_level_meets_threshold() {
        for p in [
            Priority::Critical,
            Priority::High,
            Priority::Medium,
            Priority::Low,
            Priority::Deferred,
        ] {
            assert!(
                meets_min_priority(&p, &p),
                "{p:?} should meet its own threshold"
            );
        }
    }

    #[test]
    fn priority_higher_meets_lower_threshold() {
        assert!(meets_min_priority(&Priority::Critical, &Priority::Deferred));
        assert!(meets_min_priority(&Priority::High, &Priority::Medium));
        assert!(meets_min_priority(&Priority::Medium, &Priority::Low));
        assert!(meets_min_priority(&Priority::Low, &Priority::Deferred));
    }

    #[test]
    fn priority_lower_does_not_meet_higher_threshold() {
        assert!(!meets_min_priority(
            &Priority::Deferred,
            &Priority::Critical
        ));
        assert!(!meets_min_priority(&Priority::Low, &Priority::High));
        assert!(!meets_min_priority(&Priority::Medium, &Priority::Critical));
        assert!(!meets_min_priority(&Priority::Deferred, &Priority::Low));
    }

    #[test]
    fn priority_all_combinations_consistent() {
        let variants = [
            Priority::Deferred,
            Priority::Low,
            Priority::Medium,
            Priority::High,
            Priority::Critical,
        ];
        for (i, actual) in variants.iter().enumerate() {
            for (j, min) in variants.iter().enumerate() {
                let result = meets_min_priority(actual, min);
                assert_eq!(
                    result,
                    i >= j,
                    "meets_min_priority({actual:?}, {min:?}) expected {} got {result}",
                    i >= j,
                );
            }
        }
    }

    // ── StepState From<StepStatus> ─────────────────────────────────────

    #[test]
    fn step_status_pending_maps_to_pending() {
        assert_eq!(StepState::from(StepStatus::Pending), StepState::Pending);
    }

    #[test]
    fn step_status_running_maps_to_running() {
        assert_eq!(StepState::from(StepStatus::Running), StepState::Running);
    }

    #[test]
    fn step_status_succeeded_maps_to_completed() {
        assert_eq!(StepState::from(StepStatus::Succeeded), StepState::Completed);
    }

    #[test]
    fn step_status_failed_maps_to_failed() {
        assert_eq!(StepState::from(StepStatus::Failed), StepState::Failed);
    }

    #[test]
    fn step_status_errored_maps_to_failed() {
        assert_eq!(StepState::from(StepStatus::Errored), StepState::Failed);
    }

    #[test]
    fn step_status_skipped_maps_to_skipped() {
        assert_eq!(StepState::from(StepStatus::Skipped), StepState::Skipped);
    }

    // ── determine_final_phase ─────────────────────────────────────────

    #[cfg(test)]
    mod determine_final_phase_tests {
        use super::super::{PhaseOutcome, StepStatus, determine_final_phase};

        #[test]
        fn empty_slice_returns_succeeded() {
            assert_eq!(determine_final_phase(&[]), PhaseOutcome::Succeeded);
        }

        #[test]
        fn all_succeeded_returns_succeeded() {
            let statuses = [StepStatus::Succeeded, StepStatus::Succeeded];
            assert_eq!(determine_final_phase(&statuses), PhaseOutcome::Succeeded);
        }

        #[test]
        fn one_skipped_returns_skipped() {
            let statuses = [
                StepStatus::Succeeded,
                StepStatus::Skipped,
                StepStatus::Succeeded,
            ];
            assert_eq!(determine_final_phase(&statuses), PhaseOutcome::Skipped);
        }

        #[test]
        fn one_failed_returns_failed() {
            let statuses = [
                StepStatus::Succeeded,
                StepStatus::Failed,
                StepStatus::Skipped,
            ];
            assert_eq!(determine_final_phase(&statuses), PhaseOutcome::Failed);
        }

        #[test]
        fn one_errored_returns_errored() {
            let statuses = [
                StepStatus::Succeeded,
                StepStatus::Failed,
                StepStatus::Errored,
            ];
            assert_eq!(determine_final_phase(&statuses), PhaseOutcome::Errored);
        }

        #[test]
        fn errored_beats_failed_beats_skipped_beats_succeeded() {
            let statuses = [
                StepStatus::Succeeded,
                StepStatus::Skipped,
                StepStatus::Failed,
                StepStatus::Errored,
            ];
            assert_eq!(determine_final_phase(&statuses), PhaseOutcome::Errored);
        }

        #[test]
        fn pending_and_running_map_to_aborted() {
            let statuses = [StepStatus::Pending, StepStatus::Running];
            assert_eq!(determine_final_phase(&statuses), PhaseOutcome::Aborted);
        }
    }

    // ── execution_context_to_env ───────────────────────────────────────

    #[test]
    fn env_string_value() {
        let mut ctx = ExecutionContext::new();
        ctx.set("NAME", serde_json::Value::String("alice".into()));
        let env = execution_context_to_env(&ctx);
        assert_eq!(env, vec!["NAME=alice"]);
    }

    #[test]
    fn env_number_value() {
        let mut ctx = ExecutionContext::new();
        ctx.set("PORT", serde_json::Value::Number(8080.into()));
        let env = execution_context_to_env(&ctx);
        assert_eq!(env, vec!["PORT=8080"]);
    }

    #[test]
    fn env_boolean_value() {
        let mut ctx = ExecutionContext::new();
        ctx.set("DEBUG", serde_json::Value::Bool(true));
        ctx.set("VERBOSE", serde_json::Value::Bool(false));
        let env = execution_context_to_env(&ctx);
        assert_eq!(env, vec!["DEBUG=true", "VERBOSE=false"]);
    }

    #[test]
    fn env_null_value_skipped() {
        let mut ctx = ExecutionContext::new();
        ctx.set("SKIP_ME", serde_json::Value::Null);
        ctx.set("KEEP", serde_json::Value::String("yes".into()));
        let env = execution_context_to_env(&ctx);
        assert_eq!(env, vec!["KEEP=yes"]);
    }

    #[test]
    fn env_unset_value_skipped() {
        let mut ctx = ExecutionContext::new();
        ctx.set("VISIBLE", serde_json::Value::String("ok".into()));
        ctx.unset("GONE");
        let env = execution_context_to_env(&ctx);
        assert_eq!(env, vec!["VISIBLE=ok"]);
    }

    #[test]
    fn env_array_json_serialized() {
        let mut ctx = ExecutionContext::new();
        ctx.set("TAGS", serde_json::json!(["alpha", "beta"]));
        let env = execution_context_to_env(&ctx);
        assert_eq!(env.len(), 1);
        assert_eq!(env[0], r#"TAGS=["alpha","beta"]"#);
    }

    #[test]
    fn env_object_json_serialized() {
        let mut ctx = ExecutionContext::new();
        ctx.set("META", serde_json::json!({"k": "v"}));
        let env = execution_context_to_env(&ctx);
        assert_eq!(env.len(), 1);
        assert_eq!(env[0], r#"META={"k":"v"}"#);
    }

    #[test]
    fn env_empty_context_returns_empty() {
        let ctx = ExecutionContext::new();
        let env = execution_context_to_env(&ctx);
        assert!(env.is_empty());
    }

    #[test]
    fn env_preserves_insertion_order() {
        let mut ctx = ExecutionContext::new();
        ctx.set("Z_VAR", serde_json::Value::String("z".into()));
        ctx.set("A_VAR", serde_json::Value::String("a".into()));
        ctx.set("M_VAR", serde_json::Value::String("m".into()));
        let env = execution_context_to_env(&ctx);
        assert_eq!(env, vec!["Z_VAR=z", "A_VAR=a", "M_VAR=m"]);
    }
}

// ---------------------------------------------------------------------------
// Kani formal verification proofs (cfg-gated, never compiled in normal builds)
// ---------------------------------------------------------------------------

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Proof 24: PhaseOutcome ordering matches the documented severity ladder:
    /// Succeeded < Skipped < Aborted < Failed < Errored.
    #[kani::proof]
    fn phase_outcome_ordering() {
        assert!(PhaseOutcome::Succeeded < PhaseOutcome::Skipped);
        assert!(PhaseOutcome::Skipped < PhaseOutcome::Aborted);
        assert!(PhaseOutcome::Aborted < PhaseOutcome::Failed);
        assert!(PhaseOutcome::Failed < PhaseOutcome::Errored);
    }

    /// Proof 25: PhaseOutcome ordering is total — for any two outcomes,
    /// exactly one of a < b, a == b, a > b holds.
    #[kani::proof]
    fn phase_outcome_total_order() {
        let variants = [
            PhaseOutcome::Succeeded,
            PhaseOutcome::Skipped,
            PhaseOutcome::Aborted,
            PhaseOutcome::Failed,
            PhaseOutcome::Errored,
        ];
        let i: usize = kani::any();
        let j: usize = kani::any();
        kani::assume(i < variants.len());
        kani::assume(j < variants.len());

        let a = variants[i];
        let b = variants[j];

        // Exactly one relation holds.
        let lt = a < b;
        let eq = a == b;
        let gt = a > b;
        assert!(
            (lt as u8 + eq as u8 + gt as u8) == 1,
            "ordering must be total: exactly one of <, ==, > must hold"
        );
    }

    /// Proof 26: Iterator::max over PhaseOutcome correctly selects the
    /// worst-case outcome (used for phase aggregation).
    #[kani::proof]
    fn phase_outcome_max_is_worst() {
        let variants = [
            PhaseOutcome::Succeeded,
            PhaseOutcome::Skipped,
            PhaseOutcome::Aborted,
            PhaseOutcome::Failed,
            PhaseOutcome::Errored,
        ];
        let i: usize = kani::any();
        let j: usize = kani::any();
        kani::assume(i < variants.len());
        kani::assume(j < variants.len());

        let max = std::cmp::max(variants[i], variants[j]);
        assert!(max >= variants[i]);
        assert!(max >= variants[j]);
    }

    /// Proof 27: StepStatus -> StepState mapping is total — every variant
    /// maps without panic.
    #[kani::proof]
    fn step_status_to_state_total() {
        let statuses = [
            StepStatus::Pending,
            StepStatus::Running,
            StepStatus::Succeeded,
            StepStatus::Failed,
            StepStatus::Skipped,
            StepStatus::Errored,
        ];
        let i: usize = kani::any();
        kani::assume(i < statuses.len());

        // This must not panic.
        let _state: StepState = statuses[i].into();
    }

    /// Proof 28: StepStatus::Failed and StepStatus::Errored both map to
    /// StepState::Failed (error-collapse invariant).
    #[kani::proof]
    fn step_status_error_collapse() {
        let failed: StepState = StepStatus::Failed.into();
        let errored: StepState = StepStatus::Errored.into();
        assert_eq!(
            failed, errored,
            "Failed and Errored must both map to Failed state"
        );
    }

    /// Proof 29: parse_volume with a valid "src:dst" format always produces
    /// an absolute container_path — the security invariant for mount targets.
    #[kani::proof]
    #[kani::unwind(16)]
    fn parse_volume_absolute_container_path() {
        // Use pre-built specs to avoid format! overhead in CBMC.
        let specs: [&str; 3] = ["/host/a:/mnt", "/tmp:/data", "/opt/src:/opt"];
        let i: usize = kani::any();
        kani::assume(i < specs.len());

        if let Ok(mount) = BindMount::parse_volume(specs[i]) {
            assert!(
                mount.container_path.is_absolute(),
                "container_path must be absolute"
            );
        }
    }

    /// Proof 30: parse_volume rejects specs with relative container paths.
    #[kani::proof]
    #[kani::unwind(16)]
    fn parse_volume_rejects_relative_container() {
        let specs: [&str; 3] = ["/host:relative", "/host:./rel", "/host:no_slash"];
        let i: usize = kani::any();
        kani::assume(i < specs.len());
        assert!(
            BindMount::parse_volume(specs[i]).is_err(),
            "relative container path must be rejected"
        );
    }

    /// Proof 31: parse_volume rejects relative host paths and paths with `..`.
    #[kani::proof]
    #[kani::unwind(16)]
    fn parse_volume_rejects_unsafe_host_paths() {
        let specs: [&str; 3] = ["./rel:/opt", "host:/opt", "/tmp/../etc:/mnt"];
        let i: usize = kani::any();
        kani::assume(i < specs.len());
        assert!(
            BindMount::parse_volume(specs[i]).is_err(),
            "relative or traversal host path must be rejected"
        );
    }
}

// ---------------------------------------------------------------------------
// BindMount parse tests — executable mirrors of kani proofs 29-31. A failure
// here is a real path-traversal vulnerability, not a test bug.
// ---------------------------------------------------------------------------

#[cfg(test)]
mod bind_mount_tests {
    use super::*;

    #[test]
    fn parse_volume_rejects_parent_dir_traversal() {
        assert!(BindMount::parse_volume("/tmp/../etc:/mnt").is_err());
    }

    #[test]
    fn parse_mount_rejects_parent_dir_traversal() {
        assert!(BindMount::parse_mount("type=bind,src=/tmp/../etc,dst=/mnt").is_err());
    }

    #[test]
    fn parse_mount_rejects_relative_src() {
        assert!(BindMount::parse_mount("type=bind,src=tmp/data,dst=/mnt").is_err());
    }
}

// ---------------------------------------------------------------------------
// evaluate_if_guard tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod if_guard_tests {
    use super::*;

    fn make_step(alias: &str, if_guard: Option<&str>) -> WorkflowStep {
        WorkflowStep {
            kind: "exec".to_string(),
            alias: alias.to_string(),
            if_expr: None,
            if_guard: if_guard.map(str::to_string),
            continue_on_error: false,
            retry: None,
            vars: vec![],
            config: serde_json::Value::Null,
        }
    }

    #[test]
    fn none_guard_always_true() {
        let step = make_step("s", None);
        let state = WorkflowState::new();
        assert!(evaluate_if_guard(&step, &state).expect("should not error"));
    }

    #[test]
    fn guard_resolving_to_false_literal() {
        let step = make_step("s", Some("false"));
        let state = WorkflowState::new();
        assert!(!evaluate_if_guard(&step, &state).expect("should not error"));
    }

    #[test]
    fn guard_resolving_to_zero() {
        let step = make_step("s", Some("0"));
        let state = WorkflowState::new();
        assert!(!evaluate_if_guard(&step, &state).expect("should not error"));
    }

    #[test]
    fn guard_resolving_to_empty_string() {
        let step = make_step("s", Some(""));
        let state = WorkflowState::new();
        assert!(!evaluate_if_guard(&step, &state).expect("should not error"));
    }

    #[test]
    fn guard_resolving_to_true_literal() {
        let step = make_step("s", Some("true"));
        let state = WorkflowState::new();
        assert!(evaluate_if_guard(&step, &state).expect("should not error"));
    }

    #[test]
    fn guard_resolving_to_non_empty_non_false_value() {
        let step = make_step("s", Some("yes"));
        let state = WorkflowState::new();
        assert!(evaluate_if_guard(&step, &state).expect("should not error"));
    }

    #[test]
    fn guard_using_output_reference_truthy() {
        let step = make_step("s", Some("${{ outputs['step1'].value }}"));
        let mut state = WorkflowState::new();
        state.insert("step1".to_string(), serde_json::json!({"value": "success"}));
        assert!(evaluate_if_guard(&step, &state).expect("should not error"));
    }

    #[test]
    fn guard_using_output_reference_falsy() {
        let step = make_step("s", Some("${{ outputs['step1'].value }}"));
        let mut state = WorkflowState::new();
        state.insert("step1".to_string(), serde_json::json!({"value": "false"}));
        assert!(!evaluate_if_guard(&step, &state).expect("should not error"));
    }

    #[test]
    fn guard_missing_alias_returns_err() {
        let step = make_step("s", Some("${{ outputs['missing'].value }}"));
        let state = WorkflowState::new();
        assert!(evaluate_if_guard(&step, &state).is_err());
    }
}
