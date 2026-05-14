# Plan: Promotion Engine

## Goal

Implement a multi-step workflow execution engine across minibox and crux, with an xtask promote
subcommand that gates branch promotion through dev→testing→staging→main tiers.

## Architecture

- **minibox crates affected**: `minibox-core`, `miniboxd`, `xtask`
- **crux crates affected**: `cruxx-core`, `cruxx-types`, `cruxx-script`
- **New traits/types**:
  - `minibox-core`: `WorkflowStep`, `WorkflowPhase`, `StepStatus`, `RunWorkflow` request,
    `StepRunner` trait, `StepRunnerRegistry`, `StepRetry`, `StepTimeout`, `StartFromStep`
  - `cruxx-core`: `AliasNamespace` (inter-step state), expression evaluator for if-guards,
    `DetermineFinalPhase` using worst-case step status
  - `cruxx-types`: `StepState` carrying alias propagation metadata
  - `cruxx-script`: `StepRunnerRegistry` with capability declarations
- **Data flow**:
  - CLI → `RunWorkflow` request → daemon handler → `StepRunnerRegistry::dispatch` →
    per-step `StepRunner::run` → alias propagation → streaming `WorkflowStepResult` responses
  - promote subcommand: read tier TOML → validate branch origin → run xtask gates → merge
- **Repos**: minibox (`/Users/joe/dev/minibox`), crux (`/Users/joe/dev/crux`)

## Tech Stack

- Rust 2024 edition
- `serde` (protocol serialization with `#[serde(default)]` on new fields)
- `tokio` (async handler, `spawn_blocking` for step execution)
- `proptest` (property tests for phase invariants and alias round-trips)
- `anyhow` (all error propagation)
- `xshell` (xtask promote git operations)

## Tasks

### Task 1: WorkflowStep types and RunWorkflow request

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/protocol.rs`, `crates/minibox-core/src/domain.rs`
**Run**: `cargo nextest run -p minibox-core`

Branch: `feat/issue-358-workflow-step-types` (cut from `develop`)

1. Write failing tests:

   ```rust
   // crates/minibox-core/src/protocol.rs — add to #[cfg(test)] mod tests
   #[test]
   fn workflow_step_deserializes_with_defaults() {
       let json = r#"{"type":"RunWorkflow","name":"ci","steps":[
           {"id":"build","run":"cargo build"}
       ]}"#;
       let req: DaemonRequest = serde_json::from_str(json).unwrap();
       match req {
           DaemonRequest::RunWorkflow { ref steps, .. } => {
               assert_eq!(steps[0].id, "build");
               assert!(!steps[0].continue_on_error);
               assert!(steps[0].timeout_secs.is_none());
           }
           _ => panic!("wrong variant"),
       }
   }

   #[test]
   fn workflow_phase_ord_is_total() {
       use crate::domain::WorkflowPhase;
       assert!(WorkflowPhase::Success < WorkflowPhase::Failure);
       assert!(WorkflowPhase::Failure < WorkflowPhase::Error);
   }
   ```

   Run: `cargo nextest run -p minibox-core -- workflow_step_deserializes_with_defaults`
   Expected: FAIL (types don't exist yet)

2. Implement in `crates/minibox-core/src/domain.rs`:

   ```rust
   /// Ordered severity of workflow execution outcome.
   #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
   #[serde(rename_all = "snake_case")]
   pub enum WorkflowPhase {
       Success,
       Skipped,
       Failure,
       Error,
   }

   /// Runtime status of a single step.
   #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
   #[serde(rename_all = "snake_case")]
   pub enum StepStatus {
       Pending,
       Running,
       Succeeded,
       Failed,
       Skipped,
       TimedOut,
   }

   impl StepStatus {
       pub fn is_terminal(&self) -> bool {
           matches!(self,
               StepStatus::Succeeded | StepStatus::Failed |
               StepStatus::Skipped   | StepStatus::TimedOut)
       }

       pub fn to_phase(&self) -> WorkflowPhase {
           match self {
               StepStatus::Succeeded => WorkflowPhase::Success,
               StepStatus::Skipped   => WorkflowPhase::Skipped,
               StepStatus::Failed    => WorkflowPhase::Failure,
               StepStatus::TimedOut  => WorkflowPhase::Error,
               StepStatus::Pending | StepStatus::Running => WorkflowPhase::Error,
           }
       }
   }

   /// A single step in a workflow definition.
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct WorkflowStep {
       /// Unique step identifier within the workflow.
       pub id: String,
       /// Shell command to execute.
       pub run: String,
       /// Human-readable label (optional).
       #[serde(default)]
       pub name: Option<String>,
       /// If expression evaluated before execution; `None` means always run.
       #[serde(default)]
       pub r#if: Option<String>,
       /// Alias name to export this step's output under.
       #[serde(default)]
       pub output_alias: Option<String>,
       /// Whether to continue the workflow on step failure.
       #[serde(default)]
       pub continue_on_error: bool,
       /// Per-step timeout in seconds.
       #[serde(default)]
       pub timeout_secs: Option<u64>,
       /// Retry configuration.
       #[serde(default)]
       pub retry: Option<StepRetry>,
   }
   ```

   Implement in `crates/minibox-core/src/protocol.rs` — add to `DaemonRequest`:

   ```rust
   /// Execute a named multi-step workflow.
   RunWorkflow {
       /// Workflow name for logging.
       name: String,
       /// Ordered steps to execute.
       steps: Vec<crate::domain::WorkflowStep>,
       /// Resume from this step ID (prior step outputs must be supplied).
       #[serde(default)]
       start_from: Option<String>,
       /// Prior step outputs for resumability (alias → value).
       #[serde(default)]
       prior_outputs: std::collections::HashMap<String, String>,
   },
   ```

   Add to `DaemonResponse`:

   ```rust
   /// Streaming result for a single workflow step.
   WorkflowStepResult {
       step_id: String,
       status: crate::domain::StepStatus,
       output: Option<String>,
       error:  Option<String>,
   },
   /// Workflow completed with aggregate phase.
   WorkflowComplete {
       name:  String,
       phase: crate::domain::WorkflowPhase,
   },
   ```

3. Verify:

   ```
   cargo nextest run -p minibox-core    → all green
   cargo clippy -p minibox-core -- -D warnings   → zero warnings
   ```

4. Run: `git branch --show-current`
   Expected: `feat/issue-358-workflow-step-types`
   Commit: `git commit -m "feat(minibox-core): add WorkflowStep types and RunWorkflow protocol (#358)"`

---

### Task 2: StepRunner trait and registry in minibox-core

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain.rs`
**Run**: `cargo nextest run -p minibox-core`

Branch: `feat/issue-359-step-runner-registry` (cut from `develop`, depends on Task 1 merged)

1. Write failing tests:

   ```rust
   // crates/minibox-core/src/domain.rs — #[cfg(test)] mod tests
   use super::*;
   use std::sync::Arc;

   struct EchoRunner;
   impl StepRunner for EchoRunner {
       fn name(&self) -> &str { "echo" }
       fn can_handle(&self, step: &WorkflowStep) -> bool {
           step.run.starts_with("echo ")
       }
       fn run(
           &self,
           step: &WorkflowStep,
           _state: &dyn AliasState,
       ) -> anyhow::Result<StepOutput> {
           Ok(StepOutput {
               stdout: step.run.trim_start_matches("echo ").to_string(),
               stderr: String::new(),
               exit_code: 0,
           })
       }
   }

   #[test]
   fn registry_dispatches_to_matching_runner() {
       let mut reg = StepRunnerRegistry::default();
       reg.register(Arc::new(EchoRunner));
       let step = WorkflowStep {
           id: "s1".into(), run: "echo hello".into(),
           name: None, r#if: None, output_alias: None,
           continue_on_error: false, timeout_secs: None, retry: None,
       };
       let state = NullAliasState;
       let out = reg.dispatch(&step, &state).unwrap();
       assert_eq!(out.stdout, "hello");
   }

   #[test]
   fn registry_returns_err_when_no_runner_matches() {
       let reg = StepRunnerRegistry::default();
       let step = WorkflowStep {
           id: "s2".into(), run: "unknown-cmd".into(),
           name: None, r#if: None, output_alias: None,
           continue_on_error: false, timeout_secs: None, retry: None,
       };
       let state = NullAliasState;
       assert!(reg.dispatch(&step, &state).is_err());
   }

   #[test]
   fn registry_list_returns_all_registered_names() {
       let mut reg = StepRunnerRegistry::default();
       reg.register(Arc::new(EchoRunner));
       assert!(reg.runner_names().contains(&"echo"));
   }
   ```

   Run: `cargo nextest run -p minibox-core -- registry_dispatches_to_matching_runner`
   Expected: FAIL

2. Implement in `crates/minibox-core/src/domain.rs`:

   ```rust
   /// Output produced by a single step execution.
   #[derive(Debug, Clone)]
   pub struct StepOutput {
       pub stdout: String,
       pub stderr: String,
       pub exit_code: i32,
   }

   /// Read-only view of accumulated alias state available to a runner.
   pub trait AliasState: Send + Sync {
       fn get(&self, alias: &str) -> Option<&str>;
   }

   /// Null alias state (no prior outputs).
   pub struct NullAliasState;
   impl AliasState for NullAliasState {
       fn get(&self, _alias: &str) -> Option<&str> { None }
   }

   /// A pluggable step executor.
   pub trait StepRunner: Send + Sync {
       fn name(&self) -> &str;
       fn can_handle(&self, step: &WorkflowStep) -> bool;
       fn run(&self, step: &WorkflowStep, state: &dyn AliasState) -> anyhow::Result<StepOutput>;
   }

   /// Registry of available step runners; dispatches to the first matching runner.
   #[derive(Default)]
   pub struct StepRunnerRegistry {
       runners: Vec<std::sync::Arc<dyn StepRunner>>,
   }

   impl StepRunnerRegistry {
       pub fn register(&mut self, runner: std::sync::Arc<dyn StepRunner>) {
           self.runners.push(runner);
       }

       pub fn dispatch(
           &self,
           step: &WorkflowStep,
           state: &dyn AliasState,
       ) -> anyhow::Result<StepOutput> {
           let runner = self.runners
               .iter()
               .find(|r| r.can_handle(step))
               .ok_or_else(|| anyhow::anyhow!(
                   "no runner matched step '{}' (run: {:?})", step.id, step.run
               ))?;
           runner.run(step, state)
       }

       pub fn runner_names(&self) -> Vec<&str> {
           self.runners.iter().map(|r| r.name()).collect()
       }
   }
   ```

3. Verify:

   ```
   cargo nextest run -p minibox-core    → all green
   cargo clippy -p minibox-core -- -D warnings   → zero warnings
   ```

4. Run: `git branch --show-current`
   Expected: `feat/issue-359-step-runner-registry`
   Commit: `git commit -m "feat(minibox-core): add StepRunner trait and StepRunnerRegistry (#359)"`

---

### Task 3: StepRetry and timeout model

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain.rs`
**Run**: `cargo nextest run -p minibox-core`

Branch: `feat/issue-361-step-retry-timeout` (cut from `develop`, depends on Task 1 merged)

1. Write failing tests:

   ```rust
   #[test]
   fn retry_exhausted_at_threshold() {
       let retry = StepRetry { max_attempts: 3, delay_secs: 0 };
       assert!(retry.is_exhausted(3));
       assert!(!retry.is_exhausted(2));
   }

   #[test]
   fn terminal_status_never_running() {
       for s in [StepStatus::Succeeded, StepStatus::Failed,
                 StepStatus::Skipped, StepStatus::TimedOut] {
           assert!(s.is_terminal(), "{s:?} should be terminal");
       }
       assert!(!StepStatus::Running.is_terminal());
       assert!(!StepStatus::Pending.is_terminal());
   }

   proptest::proptest! {
       #[test]
       fn error_count_at_or_above_threshold_always_fails(
           attempts in 1u32..=10,
           max in 1u32..=10,
       ) {
           let retry = StepRetry { max_attempts: max, delay_secs: 0 };
           if attempts >= max {
               proptest::prop_assert!(retry.is_exhausted(attempts));
           }
       }
   }
   ```

   Run: `cargo nextest run -p minibox-core -- retry_exhausted_at_threshold`
   Expected: FAIL

2. Implement in `crates/minibox-core/src/domain.rs`:

   ```rust
   /// Retry policy for a workflow step.
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct StepRetry {
       /// Maximum number of total attempts (first try + retries).
       pub max_attempts: u32,
       /// Seconds to wait between retries.
       #[serde(default)]
       pub delay_secs: u64,
   }

   impl StepRetry {
       /// Returns true when `attempts_made` >= `max_attempts`.
       pub fn is_exhausted(&self, attempts_made: u32) -> bool {
           attempts_made >= self.max_attempts
       }
   }
   ```

3. Verify:

   ```
   cargo nextest run -p minibox-core    → all green
   cargo clippy -p minibox-core -- -D warnings   → zero warnings
   ```

4. Run: `git branch --show-current`
   Expected: `feat/issue-361-step-retry-timeout`
   Commit: `git commit -m "feat(minibox-core): add StepRetry and timeout model (#361)"`

---

### Task 4: Alias-based state passing between workflow steps

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/domain.rs`
**Run**: `cargo nextest run -p minibox-core`

Branch: `feat/issue-360-alias-state` (cut from `develop`, depends on Tasks 1 and 2 merged)

1. Write failing tests:

   ```rust
   #[test]
   fn propagate_output_stores_and_retrieves() {
       let mut ns = AliasNamespace::default();
       ns.propagate("build_out", "artifact.tar");
       assert_eq!(ns.get("build_out"), Some("artifact.tar"));
   }

   #[test]
   fn missing_alias_returns_none() {
       let ns = AliasNamespace::default();
       assert_eq!(ns.get("nonexistent"), None);
   }

   #[test]
   fn no_silent_passthrough_on_missing_required_alias() {
       let ns = AliasNamespace::default();
       assert!(ns.require("missing_key").is_err());
   }

   proptest::proptest! {
       #[test]
       fn propagate_is_idempotent_for_non_token_steps(key in "[a-z_]{1,16}", val in ".*") {
           let mut ns = AliasNamespace::default();
           ns.propagate(&key, &val);
           ns.propagate(&key, &val);
           proptest::prop_assert_eq!(ns.get(&key), Some(val.as_str()));
       }
   }
   ```

   Run: `cargo nextest run -p minibox-core -- propagate_output_stores_and_retrieves`
   Expected: FAIL

2. Implement in `crates/minibox-core/src/domain.rs`:

   ```rust
   /// Accumulated alias→value state shared across workflow steps.
   #[derive(Debug, Default, Clone)]
   pub struct AliasNamespace {
       inner: std::collections::HashMap<String, String>,
   }

   impl AliasNamespace {
       /// Store or overwrite an alias.
       pub fn propagate(&mut self, alias: &str, value: &str) {
           self.inner.insert(alias.to_string(), value.to_string());
       }

       /// Look up an alias value.
       pub fn get(&self, alias: &str) -> Option<&str> {
           self.inner.get(alias).map(String::as_str)
       }

       /// Look up a required alias; errors if absent.
       pub fn require(&self, alias: &str) -> anyhow::Result<&str> {
           self.inner.get(alias)
               .map(String::as_str)
               .ok_or_else(|| anyhow::anyhow!("required alias '{alias}' is not set"))
       }
   }

   impl AliasState for AliasNamespace {
       fn get(&self, alias: &str) -> Option<&str> {
           AliasNamespace::get(self, alias)
       }
   }
   ```

3. Verify:

   ```
   cargo nextest run -p minibox-core    → all green
   cargo clippy -p minibox-core -- -D warnings   → zero warnings
   ```

4. Run: `git branch --show-current`
   Expected: `feat/issue-360-alias-state`
   Commit: `git commit -m "feat(minibox-core): add AliasNamespace for inter-step state passing (#360)"`

---

### Task 5: StartFromStep resumability in RunWorkflow

**Crate**: `minibox-core`, `miniboxd`
**File(s)**: `crates/minibox-core/src/protocol.rs`,
             `miniboxd/src/handler.rs` (or equivalent handler module)
**Run**: `cargo nextest run -p minibox-core -p miniboxd`

Branch: `feat/issue-362-start-from-step` (cut from `develop`, depends on Tasks 1 and 4 merged)

1. Write failing tests in `crates/minibox-core/src/protocol.rs`:

   ```rust
   #[test]
   fn start_from_step_skips_prior_steps() {
       let steps = vec![
           mk_step("a"), mk_step("b"), mk_step("c"),
       ];
       let skipped = steps_before("b", &steps);
       assert_eq!(skipped, vec!["a"]);
   }

   #[test]
   fn start_from_unknown_alias_errors() {
       let steps = vec![mk_step("a"), mk_step("b")];
       assert!(steps_before("z", &steps).is_err());
   }

   fn mk_step(id: &str) -> WorkflowStep {
       WorkflowStep { id: id.into(), run: "true".into(),
           name: None, r#if: None, output_alias: None,
           continue_on_error: false, timeout_secs: None, retry: None }
   }
   ```

   Run: `cargo nextest run -p minibox-core -- start_from_step_skips_prior_steps`
   Expected: FAIL

2. Implement a helper function in `crates/minibox-core/src/protocol.rs`:

   ```rust
   /// Returns the IDs of steps that precede `start_from` in `steps`.
   /// Returns an error if `start_from` does not match any step ID.
   pub fn steps_before<'a>(
       start_from: &str,
       steps: &'a [crate::domain::WorkflowStep],
   ) -> anyhow::Result<Vec<&'a str>> {
       let idx = steps
           .iter()
           .position(|s| s.id == start_from)
           .ok_or_else(|| anyhow::anyhow!(
               "start_from step '{start_from}' not found in workflow"
           ))?;
       Ok(steps[..idx].iter().map(|s| s.id.as_str()).collect())
   }
   ```

   In the daemon handler (locate the `RunWorkflow` arm and add resume logic):

   ```rust
   DaemonRequest::RunWorkflow { name, steps, start_from, prior_outputs } => {
       let mut alias_ns = AliasNamespace::default();
       // Load prior outputs into alias namespace
       for (k, v) in &prior_outputs {
           alias_ns.propagate(k, v);
       }
       // Determine first runnable index
       let start_idx = if let Some(ref id) = start_from {
           steps.iter().position(|s| &s.id == id)
               .ok_or_else(|| anyhow::anyhow!("start_from '{id}' not found"))?
       } else {
           0
       };
       // Emit Skipped for all pre-resume steps
       for step in &steps[..start_idx] {
           send_response(&mut writer, DaemonResponse::WorkflowStepResult {
               step_id: step.id.clone(),
               status: StepStatus::Skipped,
               output: None,
               error: None,
           }).await?;
       }
       // Execute remaining steps
       // ... (full execution loop wired in integration)
   }
   ```

3. Verify:

   ```
   cargo nextest run -p minibox-core    → all green
   cargo clippy -p minibox-core -- -D warnings   → zero warnings
   ```

4. Run: `git branch --show-current`
   Expected: `feat/issue-362-start-from-step`
   Commit: `git commit -m "feat(minibox-core,miniboxd): add StartFromStep resumability in RunWorkflow (#362)"`

---

### Task 6: cargo xtask promote subcommand

**Crate**: `xtask`
**File(s)**: `xtask/src/promote.rs` (new), `xtask/src/main.rs`
**Run**: `cargo nextest run -p xtask`

Branch: `feat/issue-363-xtask-promote` (cut from `develop`, independent of Tasks 1–5)

1. Write failing tests in `xtask/src/promote.rs`:

   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       #[test]
       fn tier_sequence_is_correct() {
           let tiers = Tier::sequence();
           assert_eq!(tiers, [Tier::Dev, Tier::Testing, Tier::Staging, Tier::Main]);
       }

       #[test]
       fn tier_next_advances_one_step() {
           assert_eq!(Tier::Dev.next(), Some(Tier::Testing));
           assert_eq!(Tier::Staging.next(), Some(Tier::Main));
           assert_eq!(Tier::Main.next(), None);
       }

       #[test]
       fn dry_run_does_not_mutate_state() {
           let plan = PromotePlan {
               from: Tier::Dev,
               to: Tier::Testing,
               dry_run: true,
               gates: vec![],
           };
           let actions = plan.collect_actions();
           assert!(actions.iter().all(|a| matches!(a, Action::DryRun(_))));
       }

       #[test]
       fn branch_not_from_develop_errors() {
           let result = validate_branch_origin("main", "feat/issue-363");
           assert!(result.is_err());
       }
   }
   ```

   Run: `cargo nextest run -p xtask -- tier_sequence_is_correct`
   Expected: FAIL

2. Implement `xtask/src/promote.rs`:

   ```rust
   //! `cargo xtask promote` — advance a branch through the tier pipeline.
   //!
   //! Tier pipeline: dev → testing → staging → main
   //! Each promotion: validate gates, fast-forward merge into target tier branch.

   use anyhow::{Context, Result, bail};
   use xshell::{Shell, cmd};

   /// Promotion tier identifiers.
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum Tier { Dev, Testing, Staging, Main }

   impl Tier {
       pub fn sequence() -> [Tier; 4] {
           [Tier::Dev, Tier::Testing, Tier::Staging, Tier::Main]
       }

       pub fn next(self) -> Option<Tier> {
           match self {
               Tier::Dev     => Some(Tier::Testing),
               Tier::Testing => Some(Tier::Staging),
               Tier::Staging => Some(Tier::Main),
               Tier::Main    => None,
           }
       }

       pub fn branch_name(self) -> &'static str {
           match self {
               Tier::Dev     => "develop",
               Tier::Testing => "testing",
               Tier::Staging => "staging",
               Tier::Main    => "main",
           }
       }
   }

   /// Promotion plan capturing options before execution.
   pub struct PromotePlan {
       pub from:    Tier,
       pub to:      Tier,
       pub dry_run: bool,
       pub gates:   Vec<String>,
   }

   /// Abstract action for dry-run reporting.
   pub enum Action {
       DryRun(String),
       Execute(String),
   }

   impl PromotePlan {
       pub fn collect_actions(&self) -> Vec<Action> {
           let desc = format!(
               "merge {} → {}", self.from.branch_name(), self.to.branch_name()
           );
           if self.dry_run {
               vec![Action::DryRun(desc)]
           } else {
               vec![Action::Execute(desc)]
           }
       }
   }

   /// Validates that the feature branch originates from `develop`.
   pub fn validate_branch_origin(base: &str, branch: &str) -> Result<()> {
       if base != "develop" {
           bail!(
               "branch '{branch}' must originate from 'develop'; found base '{base}'"
           );
       }
       Ok(())
   }

   /// Entry point wired from xtask main.
   pub fn run(sh: &Shell, dry_run: bool) -> Result<()> {
       let from_branch = cmd!(sh, "git rev-parse --abbrev-ref HEAD")
           .read()
           .context("git rev-parse")?;
       let from_branch = from_branch.trim();

       // Determine which tier we are on
       let tier = Tier::sequence()
           .iter()
           .find(|t| t.branch_name() == from_branch)
           .copied()
           .ok_or_else(|| anyhow::anyhow!(
               "current branch '{from_branch}' is not a tier branch"
           ))?;

       let to_tier = tier.next().ok_or_else(|| anyhow::anyhow!(
           "'{from_branch}' is already the final tier (main)"
       ))?;

       tracing::info!(
           from = from_branch,
           to = to_tier.branch_name(),
           dry_run,
           "promote: starting"
       );

       if dry_run {
           println!("[dry-run] would merge {} → {}", from_branch, to_tier.branch_name());
           return Ok(());
       }

       // Sync current tier to HEAD
       cmd!(sh, "git fetch origin").run().context("git fetch")?;
       cmd!(sh, "git merge --ff-only origin/{from_branch}").run()
           .context("ff-only sync")?;

       // Merge into target
       let target = to_tier.branch_name();
       cmd!(sh, "git checkout {target}").run().context("checkout target")?;
       cmd!(sh, "git merge --ff-only {from_branch}").run()
           .context("ff-only promote")?;
       cmd!(sh, "git push origin {target}").run().context("push target")?;
       cmd!(sh, "git checkout {from_branch}").run().context("restore branch")?;

       println!("promoted {from_branch} → {target}");
       Ok(())
   }
   ```

   Wire into `xtask/src/main.rs`:

   ```rust
   mod promote;
   // in match block:
   Some("promote") => {
       let dry_run = env::args().any(|a| a == "--dry-run");
       promote::run(&sh, dry_run)
   }
   ```

3. Verify:

   ```
   cargo nextest run -p xtask    → all green
   cargo clippy -p xtask -- -D warnings   → zero warnings
   ```

4. Run: `git branch --show-current`
   Expected: `feat/issue-363-xtask-promote`
   Commit: `git commit -m "feat(xtask): add promote subcommand for tier pipeline (#363)"`

---

### Task 7: crux — inter-step state propagation via alias namespace

**Crate**: `cruxx-core`, `cruxx-types`
**File(s)**: `crates/cruxx-core/src/ctx.rs`, `crates/cruxx-types/src/step.rs`
**Run**: `cargo nextest run -p cruxx-core -p cruxx-types`

Branch: `feat/issue-60-alias-namespace` (cut from `develop` in crux repo)

1. Write failing tests in `crates/cruxx-core/src/ctx.rs`:

   ```rust
   #[cfg(test)]
   mod alias_tests {
       use super::*;

       #[test]
       fn propagate_and_read_round_trip() {
           let mut ns = AliasNamespace::default();
           ns.propagate("out", "hello");
           assert_eq!(ns.get("out"), Some("hello"));
       }

       #[test]
       fn concurrent_reads_are_consistent() {
           use std::sync::{Arc, RwLock};
           let ns = Arc::new(RwLock::new(AliasNamespace::default()));
           ns.write().unwrap().propagate("k", "v");
           let handles: Vec<_> = (0..4).map(|_| {
               let ns = Arc::clone(&ns);
               std::thread::spawn(move || {
                   assert_eq!(ns.read().unwrap().get("k"), Some("v"));
               })
           }).collect();
           for h in handles { h.join().unwrap(); }
       }

       proptest::proptest! {
           #[test]
           fn sequence_of_propagates_never_corrupts_existing(
               existing_key in "[a-z]{1,8}",
               new_key in "[a-z]{1,8}",
               val in ".*",
           ) {
               proptest::prop_assume!(existing_key != new_key);
               let mut ns = AliasNamespace::default();
               ns.propagate(&existing_key, "original");
               ns.propagate(&new_key, &val);
               proptest::prop_assert_eq!(ns.get(&existing_key), Some("original"));
           }
       }
   }
   ```

   Run: `cargo nextest run -p cruxx-core -- propagate_and_read_round_trip`
   Expected: FAIL

2. Implement in `crates/cruxx-core/src/ctx.rs`:

   ```rust
   /// Alias namespace: accumulated key→value state shared across steps in a crux execution.
   #[derive(Debug, Default, Clone)]
   pub struct AliasNamespace {
       map: std::collections::HashMap<String, String>,
   }

   impl AliasNamespace {
       pub fn propagate(&mut self, key: &str, value: &str) {
           self.map.insert(key.to_string(), value.to_string());
       }

       pub fn get(&self, key: &str) -> Option<&str> {
           self.map.get(key).map(String::as_str)
       }

       pub fn require(&self, key: &str) -> anyhow::Result<&str> {
           self.map.get(key)
               .map(String::as_str)
               .ok_or_else(|| anyhow::anyhow!("alias '{key}' is not set"))
       }
   }
   ```

   Add `alias_namespace: AliasNamespace` field to `cruxx-types`'s `StepState` in
   `crates/cruxx-types/src/step.rs` with `#[serde(default)]`.

3. Verify:

   ```
   cargo nextest run -p cruxx-core -p cruxx-types    → all green
   cargo clippy -p cruxx-core -p cruxx-types -- -D warnings   → zero warnings
   ```

4. Run: `git branch --show-current`
   Expected: `feat/issue-60-alias-namespace`
   Commit: `git commit -m "feat(cruxx-core): add AliasNamespace for inter-step state propagation (#60)"`

---

### Task 8: crux — expression evaluation for step if-guards

**Crate**: `cruxx-core`
**File(s)**: `crates/cruxx-core/src/ctx.rs`
**Run**: `cargo nextest run -p cruxx-core`

Branch: `feat/issue-59-if-guard-eval` (cut from `develop` in crux repo, depends on Task 7 merged)

1. Write failing tests:

   ```rust
   #[test]
   fn field_access_resolves_from_alias_ns() {
       let mut ns = AliasNamespace::default();
       ns.propagate("build_ok", "true");
       let result = eval_if_guard("build_ok", &ns).unwrap();
       assert!(result);
   }

   #[test]
   fn missing_alias_in_guard_errors() {
       let ns = AliasNamespace::default();
       assert!(eval_if_guard("missing", &ns).is_err());
   }

   #[test]
   fn literal_true_always_passes() {
       let ns = AliasNamespace::default();
       assert!(eval_if_guard("true", &ns).unwrap());
   }

   #[test]
   fn literal_false_always_blocks() {
       let ns = AliasNamespace::default();
       assert!(!eval_if_guard("false", &ns).unwrap());
   }

   proptest::proptest! {
       #[test]
       fn no_token_guard_is_always_ok_true(key in "[a-z_]{1,8}") {
           let mut ns = AliasNamespace::default();
           ns.propagate(&key, "true");
           let r = eval_if_guard(&key, &ns);
           proptest::prop_assert!(r.is_ok());
       }
   }
   ```

   Run: `cargo nextest run -p cruxx-core -- field_access_resolves_from_alias_ns`
   Expected: FAIL

2. Implement in `crates/cruxx-core/src/ctx.rs`:

   ```rust
   /// Evaluate a step `if:` guard expression against the current alias namespace.
   ///
   /// Supported forms:
   /// - `"true"` / `"false"` — literals
   /// - `"<alias>"` — look up alias, parse as bool; error if absent or not parseable
   pub fn eval_if_guard(expr: &str, ns: &AliasNamespace) -> anyhow::Result<bool> {
       match expr.trim() {
           "true"  => return Ok(true),
           "false" => return Ok(false),
           _ => {}
       }
       let raw = ns.require(expr)
           .with_context(|| format!("if-guard references unknown alias '{expr}'"))?;
       raw.trim().parse::<bool>()
           .with_context(|| format!("if-guard alias '{expr}' = '{raw}' is not a bool"))
   }
   ```

3. Verify:

   ```
   cargo nextest run -p cruxx-core    → all green
   cargo clippy -p cruxx-core -- -D warnings   → zero warnings
   ```

4. Run: `git branch --show-current`
   Expected: `feat/issue-59-if-guard-eval`
   Commit: `git commit -m "feat(cruxx-core): add if-guard expression evaluation (#59)"`

---

### Task 9: crux — StepRunnerRegistry with capability declarations

**Crate**: `cruxx-script`
**File(s)**: `crates/cruxx-script/src/registry.rs`
**Run**: `cargo nextest run -p cruxx-script`

Branch: `feat/issue-58-step-runner-registry` (cut from `develop` in crux repo, independent of
Tasks 7–8)

1. Write failing tests in `crates/cruxx-script/src/registry.rs`:

   ```rust
   #[cfg(test)]
   mod tests {
       use super::*;

       fn make_runner(name: &'static str, caps: &[Capability]) -> Arc<dyn StepRunner> {
           struct S { name: &'static str, caps: Vec<Capability> }
           impl StepRunner for S {
               fn name(&self) -> &str { self.name }
               fn capabilities(&self) -> &[Capability] { &self.caps }
               fn can_handle(&self, step: &RunnerStep) -> bool {
                   step.run.starts_with(self.name)
               }
               fn execute(&self, _step: &RunnerStep, _ctx: &mut dyn RunnerCtx)
                   -> anyhow::Result<RunnerOutput>
               {
                   Ok(RunnerOutput { stdout: String::new(), exit_code: 0 })
               }
           }
           Arc::new(S { name, caps: caps.to_vec() })
       }

       #[test]
       fn registry_lists_capabilities_for_all_runners() {
           let mut reg = StepRunnerRegistry::new();
           reg.register(make_runner("shell", &[Capability::ShellExec]));
           reg.register(make_runner("http",  &[Capability::NetworkAccess]));
           let caps = reg.all_capabilities();
           assert!(caps.contains(&Capability::ShellExec));
           assert!(caps.contains(&Capability::NetworkAccess));
       }

       #[test]
       fn dispatch_selects_first_matching_runner() {
           let mut reg = StepRunnerRegistry::new();
           reg.register(make_runner("shell", &[]));
           let step = RunnerStep { run: "shell echo hi".into() };
           let mut ctx = NullCtx;
           let out = reg.dispatch(&step, &mut ctx).unwrap();
           assert_eq!(out.exit_code, 0);
       }

       #[test]
       fn dispatch_errors_when_no_runner_matches() {
           let reg = StepRunnerRegistry::new();
           let step = RunnerStep { run: "unknown".into() };
           let mut ctx = NullCtx;
           assert!(reg.dispatch(&step, &mut ctx).is_err());
       }

       #[test]
       fn conformance_shell_runner_declares_shell_exec_capability() {
           // Conformance: every built-in ShellRunner must declare ShellExec
           let r = ShellRunner::new();
           assert!(r.capabilities().contains(&Capability::ShellExec));
       }

       #[test]
       fn conformance_http_runner_declares_network_access() {
           let r = HttpRunner::new();
           assert!(r.capabilities().contains(&Capability::NetworkAccess));
       }
   }
   ```

   Run: `cargo nextest run -p cruxx-script -- registry_lists_capabilities_for_all_runners`
   Expected: FAIL

2. Implement in `crates/cruxx-script/src/registry.rs`:

   ```rust
   use std::sync::Arc;
   use anyhow::Result;

   /// Declared runtime capabilities of a step runner.
   #[derive(Debug, Clone, PartialEq, Eq, Hash)]
   pub enum Capability {
       ShellExec,
       NetworkAccess,
       FilesystemWrite,
       DaemonAccess,
   }

   /// Minimal step descriptor used by the registry dispatch path.
   #[derive(Debug, Clone)]
   pub struct RunnerStep {
       pub run: String,
   }

   /// Output from a runner execution.
   #[derive(Debug)]
   pub struct RunnerOutput {
       pub stdout: String,
       pub exit_code: i32,
   }

   /// Mutable execution context handed to a runner.
   pub trait RunnerCtx: Send {}

   pub struct NullCtx;
   impl RunnerCtx for NullCtx {}

   /// Pluggable step runner with capability declarations.
   pub trait StepRunner: Send + Sync {
       fn name(&self) -> &str;
       fn capabilities(&self) -> &[Capability];
       fn can_handle(&self, step: &RunnerStep) -> bool;
       fn execute(&self, step: &RunnerStep, ctx: &mut dyn RunnerCtx) -> Result<RunnerOutput>;
   }

   #[derive(Default)]
   pub struct StepRunnerRegistry {
       runners: Vec<Arc<dyn StepRunner>>,
   }

   impl StepRunnerRegistry {
       pub fn new() -> Self { Self::default() }

       pub fn register(&mut self, r: Arc<dyn StepRunner>) { self.runners.push(r); }

       pub fn dispatch(&self, step: &RunnerStep, ctx: &mut dyn RunnerCtx) -> Result<RunnerOutput> {
           self.runners
               .iter()
               .find(|r| r.can_handle(step))
               .ok_or_else(|| anyhow::anyhow!("no runner for step: {:?}", step.run))?
               .execute(step, ctx)
       }

       pub fn all_capabilities(&self) -> std::collections::HashSet<&Capability> {
           self.runners.iter().flat_map(|r| r.capabilities()).collect()
       }
   }

   // Built-in runners (minimal stubs; full impl in runner.rs)
   pub struct ShellRunner;
   impl ShellRunner { pub fn new() -> Self { ShellRunner } }
   impl StepRunner for ShellRunner {
       fn name(&self) -> &str { "shell" }
       fn capabilities(&self) -> &[Capability] { &[Capability::ShellExec] }
       fn can_handle(&self, _: &RunnerStep) -> bool { true }
       fn execute(&self, step: &RunnerStep, _: &mut dyn RunnerCtx) -> Result<RunnerOutput> {
           let output = std::process::Command::new("sh")
               .arg("-c").arg(&step.run).output()?;
           Ok(RunnerOutput {
               stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
               exit_code: output.status.code().unwrap_or(-1),
           })
       }
   }

   pub struct HttpRunner;
   impl HttpRunner { pub fn new() -> Self { HttpRunner } }
   impl StepRunner for HttpRunner {
       fn name(&self) -> &str { "http" }
       fn capabilities(&self) -> &[Capability] { &[Capability::NetworkAccess] }
       fn can_handle(&self, step: &RunnerStep) -> bool {
           step.run.starts_with("http://") || step.run.starts_with("https://")
       }
       fn execute(&self, _: &RunnerStep, _: &mut dyn RunnerCtx) -> Result<RunnerOutput> {
           anyhow::bail!("HttpRunner: not yet implemented")
       }
   }
   ```

3. Verify:

   ```
   cargo nextest run -p cruxx-script    → all green
   cargo clippy -p cruxx-script -- -D warnings   → zero warnings
   ```

4. Run: `git branch --show-current`
   Expected: `feat/issue-58-step-runner-registry`
   Commit: `git commit -m "feat(cruxx-script): add StepRunnerRegistry with capability declarations (#58)"`

---

### Task 10: crux — DetermineFinalPhase using worst-case step status

**Crate**: `cruxx-core`, `cruxx-types`
**File(s)**: `crates/cruxx-core/src/ctx.rs`, `crates/cruxx-types/src/crux_value.rs`
**Run**: `cargo nextest run -p cruxx-core -p cruxx-types`

Branch: `feat/issue-61-determine-final-phase` (cut from `develop` in crux repo, depends on
Task 7 merged)

1. Write failing tests in `crates/cruxx-core/src/ctx.rs`:

   ```rust
   use super::*;

   #[test]
   fn empty_steps_returns_success() {
       assert_eq!(determine_final_phase(&[]), FinalPhase::Success);
   }

   #[test]
   fn all_continue_on_error_still_produces_success_if_underlying_succeeded() {
       let statuses = vec![
           StepOutcome { status: OutcomeStatus::Succeeded, continue_on_error: true },
       ];
       assert_eq!(determine_final_phase(&statuses), FinalPhase::Success);
   }

   #[test]
   fn single_errored_step_without_coe_is_failure() {
       let statuses = vec![
           StepOutcome { status: OutcomeStatus::Failed, continue_on_error: false },
       ];
       assert_eq!(determine_final_phase(&statuses), FinalPhase::Failure);
   }

   proptest::proptest! {
       #[test]
       fn final_phase_at_least_as_severe_as_worst_non_coe(
           n in 1usize..=10,
       ) {
           // All non-coe failed → FinalPhase must be Failure
           let statuses: Vec<StepOutcome> = (0..n).map(|_| StepOutcome {
               status: OutcomeStatus::Failed, continue_on_error: false,
           }).collect();
           let phase = determine_final_phase(&statuses);
           proptest::prop_assert!(phase >= FinalPhase::Failure);
       }

       #[test]
       fn ord_is_total_and_consistent(a: FinalPhase, b: FinalPhase) {
           // Reflexive
           proptest::prop_assert_eq!(a.cmp(&a), std::cmp::Ordering::Equal);
           // Antisymmetric
           if a != b {
               proptest::prop_assert_ne!(a.cmp(&b), b.cmp(&a));
           }
       }
   }
   ```

   Run: `cargo nextest run -p cruxx-core -- empty_steps_returns_success`
   Expected: FAIL

2. Implement in `crates/cruxx-core/src/ctx.rs`:

   ```rust
   /// Severity-ordered outcome of a workflow.
   #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
   // Requires `proptest-derive` in dev-dependencies:
   // cruxx-core/Cargo.toml: proptest-derive = { version = "0.4", optional = false }
   // under [dev-dependencies]
   #[cfg_attr(test, derive(proptest_derive::Arbitrary))]
   pub enum FinalPhase {
       Success,
       Skipped,
       Failure,
       Error,
   }

   /// Outcome status for a single step (used only in phase calculation).
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum OutcomeStatus { Succeeded, Skipped, Failed, TimedOut }

   /// Per-step data fed into final phase calculation.
   pub struct StepOutcome {
       pub status: OutcomeStatus,
       pub continue_on_error: bool,
   }

   impl StepOutcome {
       fn effective_phase(&self) -> FinalPhase {
           if self.continue_on_error {
               // A failed step with continue_on_error does not escalate the workflow.
               return FinalPhase::Success;
           }
           match self.status {
               OutcomeStatus::Succeeded => FinalPhase::Success,
               OutcomeStatus::Skipped   => FinalPhase::Skipped,
               OutcomeStatus::Failed    => FinalPhase::Failure,
               OutcomeStatus::TimedOut  => FinalPhase::Error,
           }
       }
   }

   /// Determine the final workflow phase as the worst-case across all step outcomes.
   pub fn determine_final_phase(steps: &[StepOutcome]) -> FinalPhase {
       steps.iter()
           .map(StepOutcome::effective_phase)
           .max()
           .unwrap_or(FinalPhase::Success)
   }
   ```

3. Verify:

   ```
   cargo nextest run -p cruxx-core    → all green
   cargo clippy -p cruxx-core -- -D warnings   → zero warnings
   ```

4. Run: `git branch --show-current`
   Expected: `feat/issue-61-determine-final-phase`
   Commit: `git commit -m "feat(cruxx-core): add DetermineFinalPhase using worst-case step status (#61)"`
