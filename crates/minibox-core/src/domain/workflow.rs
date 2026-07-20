#[cfg(test)]
use super::{BindMount, ExecutionContext};
use super::{Priority, StepState};

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
