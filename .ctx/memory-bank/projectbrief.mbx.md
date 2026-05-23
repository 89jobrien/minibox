# Project brief

- **Goal**: A container runtime written in Rust with daemon/CLI split, OCI image
  pulling, Linux namespace isolation, cgroups v2 resource limits, overlay
  filesystem support, and swappable adapter suites (hexagonal architecture).
  Both a working runtime and a reference implementation for systems software
  in Rust.
- **Non-goals**: CRI compliance, Kubernetes integration, rootless support
  (blocked on user namespaces), seccomp BPF filters, MCP control surface.
- **Success criteria**: Stabilization freeze — no new features without approval.
  Linux native adapter is production-ready at v0.29.10. macOS feels native via
  smolvm (VM-backed). Full test suite (~1,467 tests) green on CI.
- **Current phase**: Stabilization + quality hardening. Rustqual refactors,
  conformance suite, test-in-vm infrastructure.

_Update this when scope changes. Paths in this memory bank: `./memory-bank/`._
