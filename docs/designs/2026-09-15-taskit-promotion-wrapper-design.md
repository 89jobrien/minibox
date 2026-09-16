---
source_sha: 98dc14c01dc5e448f6d44d0d3933d5abc194ac2c
sources:
  - CLAUDE.md
  - Cargo.toml
  - taskit.toml
  - xtask/Cargo.toml
  - xtask/src/main.rs
  - xtask/src/preflight.rs
  - xtask/src/promote.rs
  - xtask/src/protocol_drift.rs
  - xtask/schema/cli.schema.json
  - docs/core/XTASK_CLI.mbx.md
  - .github/workflows/merge.yml
generated: 2026-09-15
---

# Design: Guarded Taskit Promotion Wrapper

## Goal

Replace minibox's duplicate Git promotion implementation with a guarded compatibility wrapper
around `taskit flow auto`, preserving long-lived branch ancestry and making Taskit the only local
promotion policy engine.

## Approved Approach

Use the approved **Guarded Taskit Wrapper** approach: validate the reduced xtask command contract,
discover or interactively install Taskit, verify its flow capability, and delegate the complete
promotion unchanged.

## Context Map

### Files To Modify

| File                           | Purpose                                     | Change                                                                                         |
| ------------------------------ | ------------------------------------------- | ---------------------------------------------------------------------------------------------- |
| `xtask/src/promote.rs`         | Current direct Git promotion implementation | Replace tier/merge/CI logic with guarded Taskit delegation and deterministic ports             |
| `xtask/src/main.rs`            | Top-level CLI dispatch                      | Parse promote arguments through `promote::parse_args` and propagate Taskit's numeric exit code |
| `xtask/schema/cli.schema.json` | Machine-readable CLI contract               | Restrict promote arguments to `dry_run` and document Taskit delegation/install behavior        |
| `docs/core/XTASK_CLI.mbx.md`   | Human CLI reference                         | Document Taskit ownership, removed flags, installation policy, and failure/resume behavior     |

### Dependencies And Consumers

| File                          | Relationship                                                                             |
| ----------------------------- | ---------------------------------------------------------------------------------------- |
| `taskit.toml`                 | Canonical `main -> develop -> staging -> release -> main` flow and CI gate configuration |
| `.github/workflows/merge.yml` | Existing precedent uses merge commits for long-lived tier promotion; no change           |
| `xtask/src/preflight.rs`      | Reference pattern for process-backed ports with deterministic fakes                      |
| `xtask/src/protocol_drift.rs` | Reference pattern for `IsTerminal`-gated stdin handling                                  |
| `CLAUDE.md`                   | Declares branch pipeline and mandatory CI safety guarantees                              |
| GitHub issue `#473`           | Closed after the wrapper ships and the stale squash TODO is removed                      |

### Existing Test Coverage

`xtask/src/promote.rs` currently covers tier parsing, pipeline ordering, hop derivation, and
hand-parsed GitHub status JSON. These tests become obsolete with direct Git promotion. No current
test covers Taskit discovery, interactive installation, non-interactive refusal, capability
probing, exact argument forwarding, child exit propagation, or legacy-flag rejection.

### Reference Patterns

- `preflight::ToolProbe` demonstrates a small process port plus a real adapter and test doubles.
- `protocol_drift::read_hook_file_path` demonstrates safe `IsTerminal` handling before reading
  stdin.
- `main::parse_info_context_args` demonstrates strict rejection of duplicate, positional, and
  unknown arguments.
- `.github/workflows/merge.yml` explicitly uses `gh pr merge --auto --merge`, establishing
  ancestry-preserving promotion precedent.

### Risk

- The CLI removes `--from`, `--to`, and `--skip-ci-check`; callers receive migration guidance
  instead of silent reinterpretation.
- Promotion remains destructive. Tests must use fakes and a fake executable, never real branch
  merges.
- Installing Taskit is a networked host mutation. It requires an affirmative TTY response and is
  forbidden in dry-run and non-interactive modes.
- Taskit may stop on `release` after a failed gate. The wrapper must not restore a branch because
  Taskit's resume state is authoritative.
- The working tree already contains unrelated documentation/workflow changes; implementation must
  stage only the four files listed above.

## Crate Ownership

- **Owner crate**: `xtask` -- promotion is developer tooling and already dispatches here.
- **Affected product crates**: none.
- **New workspace crate**: none.
- **Taskit coupling**: runtime binary dependency only; no Taskit Rust crate dependency.

## Internal API

The `xtask` binary has no library API. The following items are visible only as needed by the
private `promote` module and its parent `main.rs`.

### Types

```rust
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PromoteOptions {
    dry_run: bool,
}

impl PromoteOptions {
    pub const fn new(dry_run: bool) -> Self;
    pub const fn dry_run(self) -> bool;
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TaskitProbe {
    Available,
    Missing,
    Incompatible { reason: String },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PromoteOutcome {
    exit_code: i32,
}

impl PromoteOutcome {
    pub const fn exit_code(self) -> i32;
    pub const fn is_success(self) -> bool;
}
```

`PromoteOptions` has one state because all partial-promotion and CI-bypass modes are removed.
`TaskitProbe::Missing` means process lookup returned `NotFound`; any executable that runs but does
not accept `flow auto --help` is `Incompatible`, not missing.

### Port

```rust
trait PromotionRuntime {
    fn probe_taskit_auto(&self) -> anyhow::Result<TaskitProbe>;
    fn is_interactive(&self) -> bool;
    fn confirm_install(&self) -> anyhow::Result<bool>;
    fn install_taskit(&self) -> anyhow::Result<PromoteOutcome>;
    fn run_taskit_auto(&self, options: PromoteOptions) -> anyhow::Result<PromoteOutcome>;
}

#[derive(Clone, Copy, Debug, Default)]
struct ProcessPromotionRuntime;
```

The production adapter uses `std::process::Command`, `std::io::IsTerminal`, stdin, and stderr.
`confirm_install` accepts trimmed ASCII-case-insensitive `y` or `yes`; all other input, EOF, and
read errors decline or fail without installation.

### Functions

```rust
pub fn parse_args(args: &[String]) -> anyhow::Result<PromoteOptions>;

pub fn run(root: &std::path::Path, options: PromoteOptions) -> anyhow::Result<PromoteOutcome>;

fn run_with_runtime(
    runtime: &impl PromotionRuntime,
    options: PromoteOptions,
) -> anyhow::Result<PromoteOutcome>;
```

`root` establishes the Taskit child process working directory. No Git command is issued by
`promote.rs`.

## Command Contract

Accepted invocation:

```text
cargo xtask promote
cargo xtask promote --dry-run
```

`parse_args` rejects duplicate `--dry-run`, positional values, unknown flags, and the legacy
`--from`, `--to`, and `--skip-ci-check` flags before any process call. Legacy errors direct users to
`taskit flow promote` for one-stage advancement or `taskit flow auto` for the supported full flow;
they never fall back to old behavior.

## Data Flow

1. **Parse**: `main.rs` passes all promote arguments to `promote::parse_args`.
2. **Probe**: `run_with_runtime` calls `taskit flow auto --help` through `PromotionRuntime`.
3. **Available**: a successful probe proceeds directly to Taskit execution.
4. **Missing dry-run**: return installation guidance without prompting or mutation.
5. **Missing non-interactive**: return installation guidance without reading stdin.
6. **Missing interactive**: prompt once; refusal returns guidance, acceptance runs
   `cargo install taskit --locked`.
7. **Post-install probe**: resolve and probe Taskit again; failure stops promotion.
8. **Delegate**: run `taskit flow auto`, adding `--dry-run` only when requested.
9. **Propagate**: stream inherited stdout/stderr; return `PromoteOutcome` to `main.rs`.
10. **Exit**: success returns normally; a nonzero numeric Taskit status becomes xtask's process
    exit code. Signal termination maps to exit code 1 with a diagnostic because no portable numeric
    child code exists.

## Installation Policy

- Discovery relies on normal `PATH` process resolution; no `$HOME/.cargo/bin` path is hard-coded.
- Installation command is exactly `cargo install taskit --locked` with inherited output.
- Installation requires both stdin and stderr to be terminals and an affirmative response.
- Dry-run never prompts and never installs.
- Non-interactive execution never prompts and never installs.
- Installation failure returns its numeric status and does not attempt promotion.
- A successful installation must be followed by a fresh capability probe; PATH visibility is not
  assumed.
- No Git configuration, credentials, package registry configuration, or shell startup files are
  modified.

## Exit Semantics

`main.rs` propagates `PromoteOutcome::exit_code()` only for the promote arm. It does not translate
Taskit gate failures into generic success or retry them. Errors before Taskit starts use xtask's
normal `anyhow::Result` path and exit nonzero with context.

The wrapper does not restore the original branch after Taskit starts. If Taskit stops on a CI gate,
the user follows Taskit's documented resume flow and reruns `cargo xtask promote` or
`taskit flow auto` after fixing the issue on `develop`.

## Hexagonal Boundaries

- **Port**: `PromotionRuntime` owns all process, terminal, and prompt side effects.
- **Production adapter**: `ProcessPromotionRuntime` invokes Cargo and Taskit with argument arrays,
  never shell strings.
- **Domain logic**: `parse_args` and `run_with_runtime` decide policy using only typed options,
  probe states, confirmations, and outcomes.
- **Test adapters**: deterministic fakes record calls and return configured probe/install/run
  outcomes.

No new external Rust dependency is introduced.

## Test Strategy

### Unit Tests

- No args and one `--dry-run` parse successfully.
- Duplicate dry-run, positional, unknown, and each legacy flag fail before runtime calls.
- Available Taskit runs `flow auto` exactly once.
- Dry-run forwards exactly one `--dry-run` and never installs.
- Missing Taskit in dry-run and non-interactive modes never prompts or installs.
- Interactive `y` and `yes` install; `n`, empty, invalid, EOF, and read failure do not.
- Failed installation stops before post-install probe and promotion.
- Successful installation requires a second successful probe.
- Incompatible Taskit fails without offering installation.
- Taskit success, numeric failure, and signal termination produce the specified outcomes.
- Runtime call ordering is probe -> optional prompt/install/probe -> run.

### Integration Test

Create a temporary fake `taskit` executable and invoke the promote orchestration against it. The
fake records its argument vector and exits with a configured code. Assert the working directory,
`flow auto` arguments, inherited-output behavior, and numeric exit propagation without touching a
Git repository.

### Contract Tests

- `xtask/schema/cli.schema.json` accepts only no args or `dry_run` for promote.
- `docs/core/XTASK_CLI.mbx.md` names Taskit as the owner and lists removed flags with migration
  commands.
- Source no longer contains `git checkout`, `git merge`, `gh run list`, `Tier`, `PIPELINE`, or the
  `TODO(#473)` promotion block.

## Integration Points

- `main.rs` delegates argument validation instead of partially scanning flags.
- `taskit.toml` remains unchanged and authoritative for branch names and CI gates.
- `.github/workflows/merge.yml` remains unchanged and continues using ancestry-preserving PR merge
  commits.
- CLI schema and docs change in the same implementation chain as the command contract.
- Issue `#473` closes only after code, docs, schema, and full workspace gates are committed.

## Compatibility

- `cargo xtask promote` remains the compatibility entry point.
- `--dry-run` remains supported but now delegates to Taskit.
- `--from`, `--to`, and `--skip-ci-check` are intentional breaking CLI removals with actionable
  errors.
- There is no serialized or Rust library API change.
- No feature flag is required.

## Out Of Scope

- Squash-merging long-lived tier branches.
- Implementing or maintaining direct Git promotion logic in xtask.
- Changing `.github/workflows/merge.yml` or other GitHub workflows.
- Supporting partial promotion through xtask.
- Bypassing Taskit CI gates.
- Falling back when Taskit is missing, incompatible, or fails.
- Installing Taskit in dry-run or non-interactive execution.
- Linking Taskit as a Rust dependency.
- Changing Taskit's flow implementation or resume behavior.

## Risk Checklist

- [x] Breaking CLI flags: legacy promote flags are removed with migration guidance.
- [ ] Breaking Rust API: none; `xtask` is a private binary.
- [ ] New Rust dependency: none.
- [ ] Circular dependency: none; delegation is a child process boundary.
- [ ] Feature flag required: no.
- [x] Host mutation: interactive Taskit installation is explicit and TTY-gated.
- [x] Destructive operation: Taskit retains ownership of branch mutation and CI gates.
- [x] Exit-code handling: numeric child status is propagated; signal termination is documented.
- [x] Dirty-tree risk: implementation stages only scoped files and preserves existing changes.

## Implementation Constraints

- Write a failing test before each production change.
- Stop after three failed attempts at the same test or fix.
- Before every commit run `cargo fmt --all`, re-stage exact scoped paths,
  `cargo clippy --workspace -- -D warnings`, and `cargo nextest run --workspace`.
- Run `git branch --show-current` immediately before every commit and stop on `main`.
- Never use `--no-verify`; never change Git signing configuration.
- Put temporary test/investigation artifacts under `.ctx/_WORKING_DIR/`.
