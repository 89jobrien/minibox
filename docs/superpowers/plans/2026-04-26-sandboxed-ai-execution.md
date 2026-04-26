# Dogfood-2 Sandboxed AI Code Execution — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development
> (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use
> checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enable Claude-generated scripts to run inside minibox containers instead of bare
metal. Deliver a pre-baked toolchain image (Python + Rust), an `mbx sandbox` CLI subcommand
that wraps code execution with bind mounts and resource limits, and a Claude Code hook that
redirects script execution through the sandbox.

**Architecture:** No daemon changes. Uses existing bind mount support (`-v`/`--mount`),
`ephemeral: true` streaming, and cgroup resource limits. New code is a CLI subcommand
(`mbx sandbox`) and a Dockerfile/build script for the toolchain image. The hook is a
shell script in `~/.claude/hooks/`.

**Tech Stack:** Rust (mbx CLI), Dockerfile/OCI build, `cargo nextest`, Nushell (hook script).

---

## Causal Chain

```text
T1: Build toolchain image                    (prereq for all runtime work)
  └─► T2: mbx sandbox subcommand             (CLI entry point)
        └─► T3: Resource limit defaults       (safety guardrails)
              └─► T4: Unit tests              (correctness)
                    └─► T5: Claude hook        (integration)
                          └─► T6: Docs + commit
```

**Note:** T1 is independent infrastructure. T2-T3 are sequential. T4-T5 can proceed in
parallel after T3.

---

## File Map

| Action    | Path                                       |
| --------- | ------------------------------------------ |
| Create    | `images/sandbox/Dockerfile`                |
| Create    | `images/sandbox/build.sh`                  |
| Create    | `crates/mbx/src/commands/sandbox.rs`       |
| Modify    | `crates/mbx/src/commands/mod.rs`           |
| Modify    | `crates/mbx/src/main.rs`                   |
| Create    | `crates/mbx/tests/sandbox_tests.rs`        |
| Reference | `crates/minibox-core/src/protocol.rs`      |
| Reference | `crates/minibox-core/src/client/socket.rs` |

---

## Task 1: Build toolchain image

**Files:**

- Create: `images/sandbox/Dockerfile`
- Create: `images/sandbox/build.sh`

- [ ] **Step 1: Create Dockerfile with Python + Rust toolchain**

    ```dockerfile
    FROM alpine:3.20
    RUN apk add --no-cache python3 py3-pip rust cargo gcc musl-dev
    RUN pip3 install --break-system-packages uv
    WORKDIR /workspace
    ENTRYPOINT ["/bin/sh", "-c"]
    ```

- [ ] **Step 2: Create build script that uses mbx to build/load the image**

    ```bash
    #!/usr/bin/env bash
    set -euo pipefail
    # Build via docker/podman, export as OCI tarball, load into minibox
    docker build -t minibox-sandbox:latest images/sandbox/
    docker save minibox-sandbox:latest -o /tmp/minibox-sandbox.tar
    sudo mbx load --name minibox-sandbox --tag latest /tmp/minibox-sandbox.tar
    ```

- [ ] **Step 3: Add a just recipe**

    ```just
    build-sandbox:
      bash images/sandbox/build.sh
    ```

---

## Task 2: mbx sandbox subcommand

**Files:**

- Create: `crates/mbx/src/commands/sandbox.rs`
- Modify: `crates/mbx/src/commands/mod.rs`
- Modify: `crates/mbx/src/main.rs`

**Change:** Add `mbx sandbox <script-path>` that: detects language from extension,
bind-mounts the script into the container, runs with the appropriate interpreter,
streams output back.

- [ ] **Step 1: Define CLI args**

    ```rust
    /// Run a script inside a sandboxed minibox container.
    #[derive(clap::Args)]
    pub struct SandboxArgs {
        /// Path to the script file on the host.
        script: PathBuf,
        /// Image to use (default: minibox-sandbox:latest).
        #[arg(long, default_value = "minibox-sandbox")]
        image: String,
        /// Memory limit in MB (default: 512).
        #[arg(long, default_value = "512")]
        memory_mb: u64,
        /// Timeout in seconds (default: 60).
        #[arg(long, default_value = "60")]
        timeout: u64,
        /// Extra bind mounts in host:container format.
        #[arg(long, short = 'v')]
        mount: Vec<String>,
    }
    ```

- [ ] **Step 2: Implement execute function**
    1. Detect interpreter from extension (`.py` -> `python3`, `.rs` -> `cargo-script`,
       `.sh` -> `sh`, `.nu` -> `nu`)
    2. Build `DaemonRequest::Run` with:
        - `ephemeral: true`
        - `mounts`: script file bind-mounted to `/workspace/script`
        - `memory_limit_bytes`: `memory_mb * 1024 * 1024`
        - `command`: `[interpreter, "/workspace/script"]`
    3. Stream output to terminal (reuse existing `run` streaming logic)

- [ ] **Step 3: Wire into CLI**

    Add `Sandbox(sandbox::SandboxArgs)` variant to `Commands` enum, dispatch in main.

- [ ] **Step 4: Verify compilation**

    ```bash
    cargo check -p mbx
    ```

---

## Task 3: Resource limit defaults

**Files:** `crates/mbx/src/commands/sandbox.rs`

**Change:** Enforce safety defaults: 512MB memory, 100 CPU weight, 60s timeout, no
network, no privileged mode. These are hardcoded as sandbox policy.

- [ ] **Step 1: Add timeout via tokio::time::timeout wrapping the stream loop**

- [ ] **Step 2: Force `network: None` and `privileged: false` in the request**

    Override regardless of flags — sandbox is always isolated.

- [ ] **Step 3: Add `--network` flag (default off) to opt-in to bridge networking**

    Only if the user explicitly passes `--network bridge`.

---

## Task 4: Unit tests

**Files:**

- Create: `crates/mbx/tests/sandbox_tests.rs`

- [ ] **Step 1: Test language detection**

    ```rust
    #[test]
    fn detect_python_from_extension() { ... }

    #[test]
    fn detect_rust_from_extension() { ... }

    #[test]
    fn detect_shell_from_extension() { ... }
    ```

- [ ] **Step 2: Test request construction**

    Verify the `DaemonRequest::Run` built by sandbox has correct mounts, limits,
    and `ephemeral: true`.

- [ ] **Step 3: Test timeout enforcement**

    Mock socket that never sends `ContainerStopped`. Assert sandbox returns timeout error.

- [ ] **Step 4: Run tests**

    ```bash
    cargo nextest run -p mbx -E 'test(sandbox)'
    ```

---

## Task 5: Claude Code hook

**Files:**

- Create: `~/.claude/hooks/sandbox-redirect.nu` (outside repo, user config)

- [ ] **Step 1: Write PreToolUse/Bash hook that intercepts script execution**

    ```nu
    #!/usr/bin/env nu
    # Intercept `python3 /tmp/*.py` and `bash /tmp/*.sh` and redirect to mbx sandbox
    let input = open --raw /dev/stdin | from json
    let cmd = $input.command? | default ""
    if ($cmd | str contains "/tmp/") and ($cmd =~ '\\.(py|sh|rs)$') {
        # Rewrite to mbx sandbox
        let script = ($cmd | split row " " | last)
        { command: $"sudo mbx sandbox ($script)" }
    } else {
        $input
    }
    ```

- [ ] **Step 2: Document hook installation in docs**

---

## Task 6: Docs + commit

- [ ] **Step 1: Add sandbox section to ROADMAP.md (mark as done)**

- [ ] **Step 2: Run quality gates**

    ```bash
    cargo xtask pre-commit
    cargo nextest run -p mbx -E 'test(sandbox)'
    ```

- [ ] **Step 3: Stage and commit**

    ```bash
    git add crates/mbx/src/commands/sandbox.rs crates/mbx/src/commands/mod.rs \
      crates/mbx/src/main.rs images/sandbox/ crates/mbx/tests/sandbox_tests.rs
    git commit -m "feat(cli): add mbx sandbox for AI code execution

    New subcommand wraps script execution in a minibox container with bind
    mounts, resource limits (512MB, 60s timeout), and network isolation.
    Detects language from file extension. Includes toolchain image Dockerfile.

    Ref: docs/ROADMAP.md dogfood item 2"
    ```

---

## Self-Review

**Spec coverage check:**

| Gap / objective                  | Task |
| -------------------------------- | ---- |
| Pre-baked toolchain image        | T1   |
| CLI entry point with bind mounts | T2   |
| Resource limits and timeout      | T3   |
| Language detection correctness   | T4   |
| Claude hook integration          | T5   |

**Placeholder scan:** All placeholders filled.

**Type consistency:** `DaemonRequest::Run`, `BindMount`, `NetworkMode`, `DaemonClient` —
all from `minibox_core`.
