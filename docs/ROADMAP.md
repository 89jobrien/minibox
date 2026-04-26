# Minibox Roadmap

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
