# Project brief

- **Goal**: A container runtime written in Rust with daemon/CLI split, OCI image
  pulling, Linux namespace isolation, cgroups v2 resource limits, overlay
  filesystem support, and swappable adapter suites (hexagonal architecture).
  Both a working runtime and a reference implementation for systems software
  in Rust.
- **Non-goals**: CRI compliance, rootless support (blocked on user namespaces),
  and seccomp BPF filters.
- **Success criteria**: Standing stabilization gates remain green as features land.
  Linux native adapter is production-ready at v0.33.0. macOS feels native via
  smolvm (VM-backed). Full test suite (~1,467 tests) green on CI.
- **Current phase**: Active development under the stabilization policy. The blanket
  feature freeze was lifted on 2026-08-18; quality hardening continues.

_Update this when scope changes. Paths in this memory bank: `./memory-bank/`._
