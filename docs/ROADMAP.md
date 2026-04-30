# Minibox Roadmap

## Engineering Priorities

### P0 -- Stability Gates

- **Handler coverage >= 80%**: `minibox/src/daemon/handler.rs` is at 67.5%
  function / 55% line coverage. Error path tests (image pull failure, empty
  image, registry unreachable) have the best ROI.
- **Auth policy gate**: No daemon-side policy gate on bind mounts or
  privileged mode. Any root client can mount arbitrary host paths.

### P1 -- Platform Wiring

- **krun daemon wiring**: `KrunRuntime`/`KrunRegistry`/`KrunFilesystem`/
  `KrunLimiter` adapters exist in macbox but are not yet the default on
  macOS. Goal: make krun the default macOS adapter suite.
- **Windows phase 2**: Named Pipe server, HCS/WSL2 adapter wiring in
  winbox. Currently a stub.
- **VZ Apple bug**: VZ.framework adapter blocked by VZErrorInternal on
  macOS 26 ARM64 (GH #61). Waiting on Apple fix.

### P2 -- Feature Gaps

- **Networking hardening**: Bridge networking is wired but has limited test
  coverage. Port forwarding and in-container DNS are not implemented.
- **Exec cross-platform**: `minibox exec` only works on native Linux
  adapter. GKE, Colima, and macOS adapters return errors.
- **Dockerfile parser**: `MiniboxImageBuilder` exists but there is no
  Dockerfile DSL.
- **Push/commit hardening**: `OciPushAdapter` and `overlay_commit_adapter`
  are native-only with limited test coverage.

### P3 -- Observability

- **OTEL tracing**: Spec written (`docs/superpowers/specs/`), no
  implementation yet. `otel` feature flag exists but is a no-op.
- **Metrics endpoint**: `metrics` feature flag wired but Prometheus
  endpoint coverage is minimal.

---

## Dogfooding

This section tracks ideas for using minibox to run itself and AI tooling.

### Done

- **`just dogfood`** — spins up an alpine container to validate runtime isolation, then runs `cargo xtask test-unit`. Gates the unit test suite on the container runtime proving itself healthy first.

### Planned

#### 1. MCP Server — Claude controls minibox directly

Build an MCP server that exposes minibox commands as Claude tools: `pull_image`, `run_container`, `ps`, `stop`, `rm`. Claude can then orchestrate containers in a real agent loop, exercising the daemon protocol, streaming output, and CLI end-to-end.

**Why**: highest-leverage dogfood — Claude drives the runtime, surfaces UX friction in the protocol and error messages immediately.

**Scope**: thin MCP wrapper around the Unix socket protocol (or the CLI). No new daemon features required.

---

#### 2. Sandboxed AI Code Execution

When Claude generates a script or test, run it inside a minibox container instead of bare metal. Namespace isolation + cgroups gives resource limits and a clean rootfs per execution.

**Why**: validates that the runtime is safe enough to trust with untrusted AI-generated code; also a real product use case.

**Scope**: bind mounts are shipped (`-v` / `--mount`). Remaining work: pre-baked image with
toolchain, or inject code via bind mount at run time.

---

#### 3. CI Agent — manages its own test environment via minibox

A Claude agent that:

1. Pulls a specific image via minibox
2. Runs the test suite inside the container
3. Streams stdout back to parse results
4. Cleans up after itself

**Why**: exercises the full ephemeral container lifecycle (`ephemeral: true` + streaming) and gives a real CI use case.

**Scope**: bind mounts are available. Can be implemented as a script or xtask recipe using
`mbx run -v ./src:/src minibox-tester -- cargo test`.
