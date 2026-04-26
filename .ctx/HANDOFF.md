# Handoff — minibox (2026-04-25)

**Branch:** main | **Build:** unknown | **Tests:** unknown

## Items

| ID | P | Status | Title |
|---|---|---|---|
| minibox-33 | P1 | open | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 kernel |
| minibox-46 | P1 | open | feat: PTY/stdio piping for interactive containers (#19) — plan written |
| minibox-55 | P1 | open | feat: minibox owns the full container stack on every OS |
| minibox-77 | P1 | open | chore: publish minibox-core and minibox-client to crates.io |
| minibox-agent-llm-api | P1 | open | Extend minibox-llm with infer() API, then re-land minibox-agent |
| minibox-78 | P2 | open | feat(ci): add cargo test --all-features job to CI |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts — OCI layers and bind mounts (#43) / (#66, #75) |
| minibox-52 | P2 | blocked | feat(vz): provision and start minibox-managed Linux VM via Apple VF (#40) / (#76, #84) |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent — in-VM daemon over vsock (#41) / (#78, #88) |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge — stream container stdout/stderr from VM (#42) / (#81, #93) |
| minibox-65 | P2 | blocked | [BLOCKED] feat(vz): virtiofs host-path mounts — OCI layers and bind mounts (#43) / (#75) |
| minibox-66 | P2 | blocked | [BLOCKED] feat(vz): minibox-agent — in-VM daemon over vsock (#41) / (#88) |
| minibox-67 | P2 | blocked | [BLOCKED] feat(vz): vsock I/O bridge — stream container stdout/stderr from VM (#42) / (#93) |
| minibox-7 | P2 | blocked | bug(vz): VZ.framework VZErrorInternal(code=1) on macOS 26 ARM64 |

## Log

- 20260424:192000: EOD handoff: cargo check clean. crate consolidation+plugin extraction complete. macbox/krun Phases 1-3 done. Ready for next session.
- 20260424:152652: Session 26: added init_tracing()/MINIBOX_TRACE_LEVEL (e5f178d, TDD agent); audited error system + macros (no gaps found, both tasks closed); moved dashbox to minibox-plugins (ed1def8); removed stale release artifacts (searchboxd, liblinuxbox*, libsearchbox*); new items: publish-core-crates (prereq for dockerbox move), ci-all-features. [e5f178d, ed1def8]
- 20260424:000000: Session 25: closed P1s minibox-70/75 (handler 80%, krun Phase 3). Promoted 13 P2→P1. Wave 1 orca-strait: minibox-72 (FEATURE_MATRIX SOT), minibox-73 (CI coverage gate), minibox-76 (class diagram generator), minibox-agent-llm-api (infer() API). Wave 2: minibox-58/69 already done, minibox-74 (surface tests), parallel-layer-pulls-port (minibox-oci JoinSet+Semaphore). [caa1b50, 295b7cd, ce75dab, 260dcaf, 5bfcb70]
- 20260423:232521: chore(backlog): pruned doob minibox backlog 130→100 (7 stale/closed-issue refs + 23 duplicate pairs removed); built full sequential dependency chain for all 100 items.
- 20260423:150355: chore(docs): plan audit — 29/30 done plans confirmed in git log; 1 stale; 10 missing-status plans flagged as LANDED.
