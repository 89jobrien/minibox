---
source_sha: f5481a9482fbb04690db6b7a52ee8eca9c5fe5e9
sources:
  - crates/minibox/src/daemon/handler
  - crates/minibox/src/adapters/network/bridge.rs
  - crates/minibox-domain/src/exec.rs
  - crates/minibox-core/src/image/dockerfile.rs
  - crates/minibox/src/adapters/docker_desktop.rs
  - Justfile
  - xtask/src/main.rs
  - crates/minibox/src/daemon/telemetry
  - crates/mcp
  - crates/macbox/src/vz
  - crates/macbox/tests/krun_conformance_tests.rs
  - crates/macbox/tests/krun_adapter_conformance.rs
  - crates/minibox/src/adapters/colima_commit.rs
  - crates/minibox/src/adapters/colima_push.rs
generated: 2026-09-16
---

# Minibox Roadmap

Last updated: 2026-09-19

## Engineering Priorities

### P0 -- Stability Gates

- **Handler coverage >= 80%**: met at **92.41% (207/224 functions)**. Continue adding error-path
  tests when handler behavior changes.
- **Auth policy gate**: implemented. `ContainerPolicy` and optional manifest-level
  `ExecutionPolicy` are enforced by the run handler.

### P1 -- Platform Wiring

- **Windows phase 2**: Named Pipe server, HCS/WSL2 adapter wiring in
  winbox. Currently a stub.

### Done (recently shipped)

- **unwrap → expect sweep**: All `.unwrap()` calls replaced with `.expect()`
  or `.context()?` across test and production code. `no-unwrap-in-prod` CI
  gate is now blocking.
- **Protocol-drift golden surfaces**: 5 new golden surfaces added to
  `cargo xtask` for protocol drift detection.
- **Borrow fixture verification**: `cargo xtask borrow-fixtures` subcommand
  wired and verified end-to-end.
- **Barrier-based race tests**: Daemon shared-state concurrency covered by
  barrier-synchronized race tests (#344).
- **Roundtrip property tests**: Serialized domain types covered by proptest
  roundtrip suite (#345).
- **Exhaustive small-domain tests**: Security functions covered by exhaustive
  enumeration tests (#342).
- **Stream/transport trait extraction**: `StreamReader`/`StreamWriter` traits
  extracted from handler for isolated unit testing (#343).
- **Mutation audit checklist**: Security module mutation coverage documented
  in `docs/core/SECURITY_INVARIANTS.mbx.md` (#341).
- **StateRepository persistence**: `StateRepository` wired into `DaemonState`
  for durable container state across daemon restarts (#315).
- **Handler tests split by feature**: `daemon_handler_tests` decomposed into
  per-feature files for granular CI gating (#320).
- **GKE isolation tests in CI**: `gke_adapter_isolation_tests` wired into
  `just test-integration` (#285).
- **`just install-hooks` recipe**: One-command hook installation documented
  in onboarding (#293).
- **krun daemon wiring**: `KrunRuntime`/`KrunRegistry`/`KrunFilesystem`/
  `KrunLimiter` all wired in miniboxd. 29 krun-specific conformance tests pass. krun is
  the fallback when smolvm binary is absent; smolvm remains the primary default.
- **Native bridge port forwarding and DNS**: DNAT mappings and in-container `resolv.conf`
  configuration are implemented by `BridgeNetwork`.
- **Dockerfile parser**: the supported instruction subset lives in
  `crates/minibox-core/src/image/dockerfile.rs`.
- **Image push/commit/build**: native supports all three; GKE has OCI push; Colima has
  `ColimaImagePusher`, `ColimaContainerCommitter`, and image building.
- **Observability**: OTLP tracing export and the Prometheus metrics endpoint are implemented
  behind their feature flags.
- **MCP server**: `minibox-mcp` provides the policy-gated stdio control surface.
- **VZ adapter restored**: source is present behind the `vz` feature, but remains blocked by
  `VZLinuxBootLoader` failure on current macOS.

### P2 -- Feature Gaps

- **Networking hardening**: Bridge networking, port forwarding, and DNS are wired; broader
  privileged integration coverage remains useful.
- **Exec cross-platform**: `minibox exec` only works on native Linux
  adapter. GKE, Colima, and macOS adapters return errors.
- **Push/commit hardening**: native, GKE push, and Colima paths exist; cross-adapter live
  registry coverage remains limited.

### P3 -- Observability

- **OTEL tracing**: implemented with optional OTLP/gRPC export.
- **Metrics endpoint**: implemented behind `metrics`; endpoint coverage can still improve.

---

## Dogfooding

This section tracks ideas for using minibox to run itself and AI tooling.

### Done

- **`just test-linux`** — builds the Linux dogfood image, loads it into minibox,
  and runs the test workload inside a container.

### Planned

---

#### 2. Sandboxed AI Code Execution

When Claude generates a script or test, run it inside a minibox container instead
of bare metal. Namespace isolation + cgroups gives resource limits and a clean
rootfs per execution.

**Why**: validates that the runtime is safe enough to trust with untrusted
AI-generated code; also a real product use case.

**Scope**: bind mounts are shipped (`-v` / `--mount`). Remaining work: pre-baked
image with toolchain, or inject code via bind mount at run time.

---

#### 3. CI Agent — manages its own test environment via minibox

A Claude agent that:

1. Pulls a specific image via minibox
2. Runs the test suite inside the container
3. Streams stdout back to parse results
4. Cleans up after itself

**Why**: exercises the full ephemeral container lifecycle (`ephemeral: true` +
streaming) and gives a real CI use case.

**Scope**: bind mounts are available. Can be implemented as a script or xtask
recipe using `mbx run -v ./src:/src minibox-tester -- cargo test`.
