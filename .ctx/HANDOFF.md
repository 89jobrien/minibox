# Handoff — minibox (20260423:042807)

**Branch:** feat/qemu-cross-platform | **Build:** clean | **Tests:** 51 xtask passing
**Notes:** Phase A+C of qemu-cross-platform complete. Phase B (libkrun FFI) is next separate branch.

## Open Items

| ID | Priority | Status | Title |
|----|----------|--------|-------|
| searchbox-impl | P1 | open | Complete searchbox+zoektbox implementation (Tasks 5-14) |
| minibox-70 | P1 | open | Increase daemonbox/handler.rs coverage to 80% function threshold |
| minibox-75 | P1 | open | feat(krun): KrunRuntime/Filesystem/Limiter adapters + start_krun() wiring |
| minibox-77 | P1 | open | feat(xtask): QEMU cross-platform VM runner — Phase A+C complete, Phase B pending |
| minibox-7 | P2 | blocked | bug(vz): VZ.framework VZErrorInternal(code=1) on macOS 26 ARM64 |
| minibox-33 | P2 | open | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 kernel |
| minibox-34 | P2 | open | Tier 2 mbx-dagu fixes (#31, #35, #36) |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts (#43) / (#66, #75) |
| minibox-46 | P2 | open | feat: PTY/stdio piping for interactive containers (#19) |
| minibox-52 | P2 | blocked | feat(vz): provision and start minibox-managed Linux VM via Apple VF |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent — in-VM daemon over vsock |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge |
| minibox-55 | P2 | open | feat: minibox owns the full container stack on every OS |
| minibox-58 | P2 | open | bug(ci): no CI gate required before merging release version bumps |
| minibox-65 | P2 | blocked | feat(vz): virtiofs host-path mounts (#75) |
| minibox-66 | P2 | blocked | feat(vz): minibox-agent (#88) |
| minibox-67 | P2 | blocked | feat(vz): vsock I/O bridge (#93) |
| minibox-68 | P2 | open | feat(xtask): VM image rootfs overlay directory |
| minibox-69 | P2 | open | feat(tailbox): complete tailnet integration wiring |
| minibox-72 | P2 | open | Make FEATURE_MATRIX.md single source of truth |
| minibox-73 | P2 | open | Add CI enforcement for stability checklist gates |
| minibox-74 | P2 | open | Add tests for experimental surfaces |
| minibox-76 | P2 | open | feat(docs): Rust class diagram generator script |

## Recent Log (last 5)

| Session | Date | Summary |
|---------|------|---------|
| 19 | 20260423 | feat(xtask): QEMU cross-platform Phase A+C. HostPlatform, VmRunner/VmHandle. libkrun as Phase B primary. Diagrams audited. |
| 18 | 20260423 | docs: diagrams.html crate graph rewrite, searchbox+zoektbox arch, crux pipeline. Class diagram generator scoped. |
| 17 | 20260422 | feat(searchbox+zoektbox): Tasks 1-4 via subagent TDD |
| 16 | 20260422 | Phase 1 conformance infrastructure: ConformanceCapability trait + all adapter descriptors |
| 15 | 20260421 | Pruned 3 resolved P0 items. minibox-testers Phase 1 migration landed (484 tests). |
