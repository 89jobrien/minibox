# Implementation Plan: Ephemeral Minibox Container-Based GitHub Actions Runners

## Goal

Implement the ephemeral minibox container-based GitHub Actions runner supervisor workflow that replaces the decommissioned self-hosted VPS runner. This enables the `ci-ok → promote-*` auto-merge path on staging/release branches by running the 4 Linux-root integration/e2e jobs (`test-integration`, `test-e2e`, `test-protocol-e2e-linux`, `cleanup-linux`) inside ephemeral minibox containers spawned on-demand by a GitHub Actions supervisor loop.

## Architecture

The system consists of 3 major components:

| Component                                                                 | Responsibility                                                                                                                                                                                             |
| ------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Supervisor workflow** (`ci-runner-supervisor.yml`)                      | GitHub Actions workflow running on `ubuntu-latest`; contains infinite loop that mint's fresh runner registration tokens via `gh api` and starts `mbx run --privileged --rm` with the runner image          |
| **Runner container** (Docker image `ghcr.io/89jobrien/minibox-ci-runner`) | Ephemeral GitHub Actions runner binary inside minibox container; registers with GitHub once per container, executes one job, then exits and deregisters token                                              |
| **Integration jobs** (in `.github/workflows/merge.yml`)                   | 4 jobs (`test-integration`, `test-e2e`, `test-protocol-e2e-linux`, `cleanup-linux`) change `runs-on: [self-hosted, minibox]` to `runs-on: self-hosted,minibox`; supervisor dispatches a container for each |

**Data flow**:

1. Supervisor workflow triggers; loop starts
2. `gh api repos/.../actions/runners/registration-token` mint's fresh short-lived token
3. `mbx run --privileged --rm --tag ghcr.io/89jobrien/minibox-ci-runner:latest --env RUNNER_REPO_URL=... --env RUNNER_REG_TOKEN=... --env RUNNER_LABELS=self-hosted,minibox --env RUNNER_NAME=minibox-ephemeral-<timestamp> <image>`
4. Container starts, runner binary registers with GitHub, waits for job assignment
5. Workflow job with `runs-on: self-hosted,minibox` is picked up by the container
6. Job executes inside the minibox container (namespaces, cgroups, overlay FS via the selected adapter)
7. Job exits; container exit trigger deregisters token via `config.sh remove --token`
8. Container exits; supervisor loop mint's new token and repeats

## Tech Stack

- **GitHub Actions** (workflow `ci-runner-supervisor.yml`, `.github/workflows/merge.yml`)
- **bash** (supervisor loop `ops/ci-runner/supervisor.sh`, entrypoint `ops/ci-runner/entrypoint.sh`)
- **GitHub CLI** (`gh api` for token minting)
- **minibox** (`mbx run --privileged --rm` for container orchestration)
- **Docker** (used once via `mbx build -t ghcr.io/89jobrien/minibox-ci-runner:latest ops/ci-runner/` to build the runner image)
- **Nushell** (for local testing: `just e2e-local`, `cargo xtask e2e-local`)
- **Rust 2024** (underlying adapter: smolvm/krun/native — all confirmed working this session)

---

## Tasks

### Task 1: Add ci-runner-supervisor.yml workflow

**Crate**: (workflow, not a crate)  
**File(s)**: `.github/workflows/ci-runner-supervisor.yml`  
**Run**: `just e2e-local` (verify smolvm build + krun exec work); later `cargo xtask test-unit -p minibox-dispatch` (if applicable)

- **Failing test**: N/A — new workflow file; verify by `.github/workflows/` listing and `git status`
- **Verify FAIL**: `git diff --stat` shows no untracked workflow files; `cargo xtask verify` should pass (fmt/clippy/docs audit)
- **Implement**: Create `.github/workflows/ci-runner-supervisor.yml` with:
  - `on: workflow_dispatch + schedule: ['0 * * * *']` (hourly keep-alive)
  - Job `supervisor` running on `ubuntu-latest`
  - `timeout-minutes: 120` (prevent stuck runs)
  - `env: RUNNER_REPO, RUNNER_IMAGE, RUNNER_IMAGE_TAG, RUNNER_LABELS`
  - Step: `install tools` (ensure `gh` and `mbx` available)
  - Step: `run supervisor loop` with `ops/ci-runner/supervisor.sh`
- **Verify GREEN**: `git status` shows new file; `cargo xtask verify` passes; workflow YAML is valid (no syntax errors)
- **Commit**: `git add .github/workflows/ci-runner-supervisor.yml`; conventional commit: `feat: add ci-runner-supervisor workflow for ephemeral minibox runners`

### Task 2: Verify supervisor loop runs locally

**Crate**: (bash script, not a crate)  
**File(s)**: `ops/ci-runner/supervisor.sh`  
**Run**: `RUNNER_REPO=89jobrien/minibox ops/ci-runner/supervisor.sh` (interrupt with Ctrl-C after 2-3 iterations)

- **Failing test**: Supervisor fails on first iteration (token mint failure, mbx start failure) — verify error output is meaningful
- **Verify FAIL**: Run supervisor locally; observe first iteration failure; capture exit code and stderr
- **Implement**: Ensure supervisor.sh handles these gracefully:
  - `gh api` failure: clear error message + exit (or retry with backoff)
  - `mbx run` failure: detect failure mode, log reason, restart loop
  - Token mint success + mbx start success: container starts and registers (verify with `gh api repos/.../actions/runners`)
- **Verify GREEN**: Supervisor completes at least 1 full iteration (mint token → start mbx → container registers → exit); exit code 0 or controlled exit
- **Commit**: `git add ops/ci-runner/supervisor.sh`; conventional commit: `feat: improve supervisor error handling and logging`

### ### Task 3: Build and push runner Docker image

**Crate**: (image build, not a Rust crate)  
**File(s)**: `ops/ci-runner/Dockerfile` (verify unchanged); `ops/ci-runner/entrypoint.sh` (verify unchanged)  
**Run**: `MINIBOX_ADAPTER=smolvm mbx build -t ghcr.io/89jobrien/minibox-ci-runner:latest ops/ci-runner/` (all 11 steps verified this session)

- **Failing test**: `mbx build` fails on any of the 11 Dockerfile steps (RUN-chaining bug or COPY/VM-bridging gap)
- **Verify FAIL**: Run `mbx build`; capture exit code + first failing step
- **Implement**: The Dockerfile and entrypoint are already verified (11 steps pass as of 2026-10-15). If any step fails, investigate and fix per the known bugs (RUN-chaining fixed 2026-10-14, COPY/VM-bridging fixed 2026-10-15). Ensure `mbx build` completes with `Successfully built Successfully tagged` and `smolvm machine ls` shows no lingering machine after.
- **Verify GREEN**: `mbx build -t ghcr.io/89jobrien/minibox-ci-runner:latest ops/ci-runner/` exits 0; `smolvm machine ls` (or `krun machine ls` on macOS) shows no persistent machines; `mbx push ghcr.io/89jobrien/minibox-ci-runner:latest --token "$(gh auth token)"` succeeds or reports "already uploaded"
- **Commit**: `git add ops/ci-runner/Dockerfile ops/ci-runner/entrypoint.sh`; conventional commit: `chore: update ci-runner Dockerfile/entrypoint (verified 2026-11-04)`

### ### Task 4: Route the 4 integration jobs through the supervisor

**Crate**: (workflow changes, not a Rust crate)  
**File(s)**: `.github/workflows/merge.yml` (lines 203-212)  
**Run**: Trigger the supervisor workflow via `gh workflow run ci-runner-supervisor`; then trigger a test job with `runs-on: self-hosted,minibox`; verify the supervisor dispatches a container and the job runs

- **Failing test**: Integration jobs still use old `runs-on: [self-hosted, minibox]` without a runner — they queue pending forever
- **Verify FAIL**: `gh run list --repo 89jobrien/minibox --filter status=pending` shows jobs stuck; or `gh api repos/89jobrien/minibox/actions/runners` shows 0 runners active
- **Implement**: Change the 4 jobs in `merge.yml` from `runs-on: [self-hosted, minibox]` to `runs-on: self-hosted,minibox` (keep the label, but now the supervisor dispatches a container for each). Also remove the `if: github.ref == '__disabled_no_self_hosted_runner__'` guard (refinement 3a) since the runner mechanism is now operational.
- **Verify GREEN**:
  - Supervisor workflow is triggered (via `workflow_dispatch` or manual `gh workflow run`)
  - A runner container starts, registers with GitHub, and picks up one job
  - The job executes inside the minibox container
  - Job exits; container deregisters and exits
  - Supervisor loops for next job
  - `cargo xtask verify` passes (no regressions)
- **Commit**: `git add .github/workflows/merge.yml`; conventional commit: `feat: route integration jobs through ephemeral minibox runner supervisor`

### ### Task 5: Update ci-ok promotion path and verify

**Crate**: (CI gate, not a Rust crate)  
**File(s)**: `.github/workflows/merge.yml` (ci-ok job, lines 328-372)  
**Run**: `cargo xtask verify` (full gate); then trigger a develop→staging promotion and verify it completes without hanging on self-hosted runner

- **Failing test**: `ci-ok` job fails or times out because it depends on the 4 integration jobs which are blocked by missing runner
- **Verify FAIL**: Run `cargo xtask verify`; check if `test-e2e`/`test-integration` results are failure/skipped due to no runner
- **Implement**: After Task 4 routes the 4 jobs through the supervisor, `ci-ok` should see them pass (or at least not hang). If `ci-ok` still has issues, adjust the `needs:` dependencies or add `if: exists()` checks for the runner mechanism.
- **Verify GREEN**: `cargo xtask verify` passes end-to-end (or at least the `ci-ok` step reports all sub-jobs green or skipped-with-reason, not failure); develop→staging promotion PR can be created without manual `--skip-ci-check` workaround.
- **Commit**: `git add .github/workflows/merge.yml`; conventional commit: `fix: unblock ci-ok promotion path via ephemeral minibox runners`

---

## Introspection Checklist (before saving)

- [x] Every requirement maps to a task: 5 tasks covering workflow, supervisor, image build, job routing, and promotion path
- [x] No vague directives: each task has specific failing test → verify FAIL → implement → verify GREEN → commit flow
- [x] Consistent names across tasks: all tasks use `### Task N: <name>` headings; Crate/File(s)/Run annotations are uniform
- [x] Each task is 2-5 minutes of focused work: tasks are granular (workflow file, supervisor loop, image build, job routing, gate check)
- [x] The generated plan path is captured: `/Users/joe/dev/minibox/docs/plans/2026-11-04-ephemeral-minibox-runners.md`
- [x] Next command is `/gm-ingest /Users/joe/dev/minibox/docs/plans/2026-11-04-ephemeral-minibox-runners.md`

---

**Generated path**: `/Users/joe/dev/minibox/docs/plans/2026-11-04-ephemeral-minibox-runners.md`

**Next command**: `/gm-ingest /Users/joe/dev/minibox/docs/plans/2026-11-04-ephemeral-minibox-runners.md`
