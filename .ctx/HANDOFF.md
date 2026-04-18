# Handoff — minibox (2026-04-18)

**Branch:** main | **Build:** ok | **Tests:** 438/438 pass (nextest)

## Items

| ID | P | Status | Title |
|---|---|---|---|
| minibox-46 | P2 | open | feat: PTY/stdio piping — domain types landed, fork/exec deferred |
| minibox-55 | P2 | open | feat: minibox owns the full container stack on every OS |
| minibox-58 | P2 | open | bug(ci): no CI gate for release version bumps (#55) |
| minibox-33 | P2 | open | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 |
| minibox-34 | P2 | open | Tier 2 mbx-dagu fixes (#31, #35, #36) |
| minibox-68 | P2 | open | feat(xtask): VM image rootfs overlay directory (~/.mbx/vm/overlay/) |
| minibox-69 | P2 | open | feat(tailbox): complete tailnet integration wiring (uncommitted changes) |
| minibox-7  | P2 | blocked | bug(vz): VZErrorInternal(code=1) on macOS 26 ARM64 — Apple bug |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts (#66, #75) |
| minibox-52 | P2 | blocked | feat(vz): provision Linux VM via Apple VF (#76, #84) |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent in-VM over vsock (#78, #88) |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge (#81, #93) |
| minibox-65 | P2 | blocked | feat(vz): virtiofs host-path mounts (#75) |
| minibox-66 | P2 | blocked | feat(vz): minibox-agent (#88) |
| minibox-67 | P2 | blocked | feat(vz): vsock I/O bridge (#93) |

## Log

- 2026-04-18: fix(xtask) set_current_dir before Shell::new() — survives deleted worktree CWDs.
  chore: bump 0.18.1→0.18.2, pushed. [6fb238e, bf580df]
- 2026-04-17: tailbox crate: Tailscale networking integration — TailnetNetwork adapter,
  TailnetConfig, auth key chain (1Password/env/file), gateway caching, per-container
  setup/cleanup. miniboxd wired with tailnet feature flag. 11 commits (9bd9de8..67896ac).
- 2026-04-17: orca-strait Wave 1+2 — TestBackendDescriptor (#69), xtask ToolProbe preflight
  (#95), conformance tests commit/build/push (#62,#67,#71), PtyConfig+PtyAllocator (#83).
  Closed T-arch GH issues #101/#102/#105/#106. 438/438 tests pass.
- 2026-04-17: minibox-69 (ContainerState unification), minibox-26 (nsenter exec). [2668537, 08e07f8]
- 2026-04-17: Test coverage sprint — escape detection, GKE adapter suite, lifecycle failures,
  Colima conformance; minibox-oci extracted. [a4ccf4c, b3c8b5f, 92578fd]
