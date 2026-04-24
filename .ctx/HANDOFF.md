# Handoff — minibox (2026-04-24)

**Branch:** main | **Build:** unknown | **Tests:** unknown

## Items

| ID | P | Status | Title |
|---|---|---|---|
| minibox-33 | P1 | open | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 kernel |
| minibox-46 | P1 | open | feat: PTY/stdio piping for interactive containers (#19) — plan written |
| minibox-55 | P1 | open | feat: minibox owns the full container stack on every OS |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts — OCI layers and bind mounts (#43) / (#66, #75) |
| minibox-52 | P2 | blocked | feat(vz): provision and start minibox-managed Linux VM via Apple VF (#40) / (#76, #84) |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent — in-VM daemon over vsock (#41) / (#78, #88) |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge — stream container stdout/stderr from VM (#42) / (#81, #93) |
| minibox-65 | P2 | blocked | [BLOCKED] feat(vz): virtiofs host-path mounts — OCI layers and bind mounts (#43) / (#75) |
| minibox-66 | P2 | blocked | [BLOCKED] feat(vz): minibox-agent — in-VM daemon over vsock (#41) / (#88) |
| minibox-67 | P2 | blocked | [BLOCKED] feat(vz): vsock I/O bridge — stream container stdout/stderr from VM (#42) / (#93) |
| minibox-7 | P2 | blocked | bug(vz): VZ.framework VZErrorInternal(code=1) on macOS 26 ARM64 |

## Log

- 20260424:000000: Session 25: closed P1s minibox-70/75 (handler 80%, krun Phase 3). Promoted 13 P2→P1. Wave 1 orca-strait: minibox-72 (FEATURE_MATRIX SOT), minibox-73 (CI coverage gate), minibox-76 (class diagram generator), minibox-agent-llm-api (infer() API). Wave 2: minibox-58/69 already done, minibox-74 (surface tests), parallel-layer-pulls-port (minibox-oci JoinSet+Semaphore). [caa1b50, 295b7cd, ce75dab, 260dcaf, 5bfcb70]
- 20260423:232521: chore(backlog): pruned doob minibox backlog 130→100 (7 stale/closed-issue refs + 23 duplicate pairs removed); built full sequential dependency chain for all 100 items — 71 todos wired with blocks/blocked_by in topological order covering VZ, mac dogfood, conformance, handler/CI, linuxbox rename, run_review, and PTY chains.
- 20260423:150355: chore(docs): plan audit — 29/30 done plans confirmed in git log; 1 stale (vps-automation-safety, no commit evidence); 10 missing-status plans flagged as LANDED (ci-agent-hardening, llm-timeouts-retries, minibox-llm, sandbox-tests, tailnet-integration, test-linux-dogfood, crux-minibox-integration, minibox-testers-migration, searchbox, qemu-cross-platform); pty-stdio-piping partial (domain types only); conformance-phase2 in progress; testing-strategy-expansion has no evidence. No code changes this session.
- 20260423:185322: chore(test): full test coverage sweep — proptest suites for minibox-agent/tailbox/zoektbox, conformance tests for AgentError and TailnetConfig, 3 new fuzz targets (parse_www_authenticate, parse_manifest, agent_message_decode), READMEs for 7 crates (mbx updated, minibox/minibox-agent/minibox-testers/tailbox/zoektbox/searchbox added). All 103 tests pass, fuzz workspace checks clean.
- 20260423:081051: docs+chore: audited and fixed 6 diagrams (container-lifecycle, crate-graph, env-var-flow, hexagonal, mbxctl-sse, platform-selection); converted all to standalone HTML with Mermaid.js; cleaned 14 local + 5 remote merged branches; merged feat/docs-cleanup and feat/repo-hygiene; reverted feat/minibox-agent-error (LLM API incompatible) and documented design in docs/minibox-agent-design.md; aborted feat/parallel-layer-pulls merge (stale crate path) and documented design in docs/parallel-layer-pulls-design.md; applied pre-session stash (minibox-llm simplification, preflight cleanup, socket rename miniboxd.sock→minibox.sock). [3528b40, 6fbd81d, bc71176, d8809b1, c20ef80]
