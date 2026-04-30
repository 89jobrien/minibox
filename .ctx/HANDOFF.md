# Handoff — minibox (2026-04-30)

**Branch:** main | **Build:** clean | **Tests:** 9/9 typestate pass; cargo check clean
Wave 2 of update/upgrade fully landed on main. Bench dashboard rewritten with
Chart.js history tracking. Typestate lifecycle module added to minibox-core
(compile-time container state machine, 9 tests). Ready for next->stable promotion
once CI is green on main.


## Items

| ID | P | Status | Title |
|---|---|---|---|
| minibox-33 | P1 | open | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 kernel |
| minibox-46 | P1 | open | feat: PTY/stdio piping for interactive containers (#19) — plan written |
| minibox-55 | P1 | open | feat: minibox owns the full container stack on every OS |
| minibox-77 | P1 | open | chore: promote to stable and cut v0.22.0 release tag |
| minibox-update-upgrade | P1 | open | feat: mbx update + mbx upgrade — restart support stubbed (Wave 3) |
| minibox-agent-llm-api | P1 | blocked | Extend minibox-llm with infer() API, then re-land minibox-agent |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts — OCI layers and bind mounts (#43) / (#66, #75) |
| minibox-52 | P2 | blocked | feat(vz): provision and start minibox-managed Linux VM via Apple VF (#40) / (#76, #84) |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent — in-VM daemon over vsock (#41) / (#78, #88) |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge — stream container stdout/stderr from VM (#42) / (#81, #93) |
| minibox-7 | P2 | blocked | bug(vz): VZ.framework VZErrorInternal(code=1) on macOS 26 ARM64 |

## Log

- 20260429:042914: Session 38: landed Wave 2 of update/upgrade (handle_update handler, mbx update CLI, sentinel fixes, CI e2e tests, v0.22.0 bump); added rich bench dashboard (xtask bench rewrite + Chart.js HTML); added compile-time typestate lifecycle module (minibox-core::typestate, 9 tests); explored container ecosystem comparison (tini/conmon/containerd/ctop gaps identified). [9574520, d3459f4, e08a7dc, 277dbf2, 761fd26, 5a4aadf]
- 20260428:124518: Session 37: added mbx update (image refresh) + mbx upgrade (self-update) commands — Wave 1 landed (protocol types, upgrade CLI); Wave 2 agents dispatched (handle_update handler, update CLI). [54df5d0, 1bc83c5]
- 20260428:121625: Session 36: resolved all 6 council recommendations for multi-platform image pull — fixed registry routing bypass (c669ca8), hardened TargetPlatform::parse, fixed proot stderr capture, wired run --platform end-to-end, restored 3 CI jobs, added platform-aware pull handler tests. [c669ca8, f423e19, 49fc9a1, 73703f8, 93b4814, ade65b9, eb6bef2]
- 20260428:062906: Session 35: orca-strait multi-platform image pull — TargetPlatform type, find_platform(), NoPlatformManifest error, RegistryClient::with_platform(), protocol platform field, --platform CLI flag, GHCR adapter update. 2 commits landed, mbx in working tree. [b8720ac, 4e4bdba]
- 20260428:080155: Session 34: implemented minibox-memory crate in minibox-plugins — hexagonal MemoryStore+Embedder ports, InMemoryStore+TursoStore adapters, InfraMemory service (L0/L1/L2), MemorySearcher RRF hybrid search. 26 tests passing. Fixed obfsck pre-commit exclusions. [5b4c4d8]
