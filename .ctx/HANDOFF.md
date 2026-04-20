# Handoff — minibox (20260420:050814)

**Branch:** main | **Build:** ok (cargo check --workspace) | **Tests:** 452 passing
**Notes:** Tailbox src changes uncommitted (deferred pending macbox compile fix review).

## Items

| ID | P | Status | Title |
|---|---|---|---|
| minibox-70 | P1 | open | Increase daemonbox/handler.rs coverage to 80% function threshold |
| minibox-71 | P1 | open | Add integration tests for every wired adapter path (native, gke, colima) |
| minibox-46 | P2 | open | feat: PTY/stdio piping for interactive containers (#19) — plan written |
| minibox-55 | P2 | open | feat: minibox owns the full container stack on every OS |
| minibox-58 | P2 | open | bug(ci): no CI gate required before merging release version bumps (#55) |
| minibox-68 | P2 | open | feat(xtask): VM image rootfs overlay directory (~/.mbx/vm/overlay/) |
| minibox-69 | P2 | open | feat(tailbox): complete tailnet integration wiring |
| minibox-72 | P2 | open | Make FEATURE_MATRIX.md single source of truth |
| minibox-73 | P2 | open | Add CI enforcement for stability checklist gates |
| minibox-74 | P2 | open | Add tests for experimental surfaces: bridge networking, exec, persistence |
| minibox-33 | P2 | open | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 kernel |
| minibox-34 | P2 | open | Tier 2 mbx-dagu fixes (#31, #35, #36) |
| minibox-7  | P2 | blocked | bug(vz): VZ.framework VZErrorInternal(code=1) on macOS 26 ARM64 |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts (#66, #75) |
| minibox-52 | P2 | blocked | feat(vz): provision and start minibox-managed Linux VM (#76, #84) |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent — in-VM daemon over vsock (#78, #88) |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge (#81, #93) |
| minibox-65 | P2 | blocked | feat(vz): virtiofs host-path mounts (#75) |
| minibox-66 | P2 | blocked | feat(vz): minibox-agent — in-VM daemon over vsock (#88) |
| minibox-67 | P2 | blocked | feat(vz): vsock I/O bridge (#93) |

## Log

- 20260420:050814: Session: fixed ANTHROPIC_API_KEY in ~/dev/.env. Council report 271aaba → 10 GH issues (#128-#137). Forge agent closed #128. Review fixes: AgentError variants, CLAUDE.md, HANDOFF tailbox context, checklist attribution. Version bump 0.18.2→0.19.0, pushed 21 commits. 452 tests passing. [96b026a, 7fa04e5, 87f6f20, 2e62bf0, 271aaba]
- 2026-04-18: fix(xtask): CWD before Shell::new(). chore: bump 0.18.1→0.18.2. pushed to origin/main. [6fb238e, bf580df]
- 2026-04-17: tailbox integration complete: TailnetNetwork adapter, TailnetConfig, auth chain, gateway IP caching. miniboxd wired with tailnet feature flag. [67896ac..6aa8175]
- 2026-04-17: orca-strait Wave 1+2: BackendDescriptor, xtask ToolProbe, conformance tests, PtyConfig+PtyAllocator. 438/438 tests pass. [74bdabb..64fe60e]
- 2026-04-17: Test coverage sprint: escape detection, GKE adapter suite, lifecycle failures, Colima conformance. minibox-oci extracted. [a4ccf4c..92578fd]
