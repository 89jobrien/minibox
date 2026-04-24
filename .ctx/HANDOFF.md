# Handoff — minibox (2026-04-23)

**Branch:** main | **Build:** unknown | **Tests:** unknown

## Items

| ID                        | P   | Status  | Title                                                                                                              |
| ------------------------- | --- | ------- | ------------------------------------------------------------------------------------------------------------------ |
| minibox-70                | P1  | open    | Increase daemonbox/handler.rs coverage to 80% function threshold                                                   |
| minibox-75                | P1  | open    | feat(krun): KrunRuntime/Filesystem/Limiter adapters + start_krun() wiring — cross-platform (macOS HVF + Linux KVM) |
| minibox-33                | P2  | open    | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 kernel                                          |
| minibox-34                | P2  | open    | Tier 2 mbx-dagu fixes (#31, #35, #36)                                                                              |
| minibox-46                | P2  | open    | feat: PTY/stdio piping for interactive containers (#19) — plan written                                             |
| minibox-55                | P2  | open    | feat: minibox owns the full container stack on every OS                                                            |
| minibox-58                | P2  | open    | bug(ci): no CI gate required before merging release version bumps (#55)                                            |
| minibox-68                | P2  | open    | feat(xtask): VM image rootfs overlay directory (~/.mbx/vm/overlay/)                                                |
| minibox-69                | P2  | open    | feat(tailbox): complete tailnet integration wiring — miniboxd feature flag + auth chain + gateway routing          |
| minibox-72                | P2  | open    | Make FEATURE_MATRIX.md single source of truth — trim README/CLAUDE to summaries                                    |
| minibox-73                | P2  | open    | Add CI enforcement for stability checklist gates                                                                   |
| minibox-74                | P2  | open    | Add tests for experimental surfaces: bridge networking, exec, persistence                                          |
| minibox-76                | P2  | open    | feat(docs): Rust class diagram generator script for docs/diagrams.html                                             |
| minibox-agent-llm-api     | P2  | open    | Extend minibox-llm with infer() API, then re-land minibox-agent                                                    |
| parallel-layer-pulls-port | P2  | open    | Port feat/parallel-layer-pulls to linuxbox (crate rename required)                                                 |
| minibox-43                | P2  | blocked | feat(vz): virtiofs host-path mounts — OCI layers and bind mounts (#43) / (#66, #75)                                |
| minibox-52                | P2  | blocked | feat(vz): provision and start minibox-managed Linux VM via Apple VF (#40) / (#76, #84)                             |
| minibox-53                | P2  | blocked | feat(vz): minibox-agent — in-VM daemon over vsock (#41) / (#78, #88)                                               |
| minibox-54                | P2  | blocked | feat(vz): vsock I/O bridge — stream container stdout/stderr from VM (#42) / (#81, #93)                             |
| minibox-65                | P2  | blocked | [BLOCKED] feat(vz): virtiofs host-path mounts — OCI layers and bind mounts (#43) / (#75)                           |
| minibox-66                | P2  | blocked | [BLOCKED] feat(vz): minibox-agent — in-VM daemon over vsock (#41) / (#88)                                          |
| minibox-67                | P2  | blocked | [BLOCKED] feat(vz): vsock I/O bridge — stream container stdout/stderr from VM (#42) / (#93)                        |
| minibox-7                 | P2  | blocked | bug(vz): VZ.framework VZErrorInternal(code=1) on macOS 26 ARM64                                                    |

## Log

- 20260424: refactor(minibox): absorbed crates/linuxbox into crates/minibox — 78 files migrated,
  linuxbox crate deleted, pre-commit gate clean, committed 03774be on feat/tracing-init.
  init_tracing() (MINIBOX_TRACE_LEVEL), security regression tests, preflight helpers also landed
  this session. Branch needs PR → main. parallel-layer-pulls-port HANDOFF item: dependency on
  linuxbox now resolved (content is in crates/minibox).
- 20260423:232521: chore(backlog): pruned doob minibox backlog 130→100 (7 stale/closed-issue refs + 23 duplicate pairs removed); built full sequential dependency chain for all 100 items — 71 todos wired with blocks/blocked_by in topological order covering VZ, mac dogfood, conformance, handler/CI, linuxbox rename, run_review, and PTY chains.
- 20260423:150355: chore(docs): plan audit — 29/30 done plans confirmed in git log; 1 stale (vps-automation-safety, no commit evidence); 10 missing-status plans flagged as LANDED (ci-agent-hardening, llm-timeouts-retries, minibox-llm, sandbox-tests, tailnet-integration, test-linux-dogfood, crux-minibox-integration, minibox-testers-migration, searchbox, qemu-cross-platform); pty-stdio-piping partial (domain types only); conformance-phase2 in progress; testing-strategy-expansion has no evidence. No code changes this session.
- 20260423:185322: chore(test): full test coverage sweep — proptest suites for minibox-agent/tailbox/zoektbox, conformance tests for AgentError and TailnetConfig, 3 new fuzz targets (parse_www_authenticate, parse_manifest, agent_message_decode), READMEs for 7 crates (mbx updated, minibox/minibox-agent/minibox-testers/tailbox/zoektbox/searchbox added). All 103 tests pass, fuzz workspace checks clean.
- 20260423:081051: docs+chore: audited and fixed 6 diagrams (container-lifecycle, crate-graph, env-var-flow, hexagonal, mbxctl-sse, platform-selection); converted all to standalone HTML with Mermaid.js; cleaned 14 local + 5 remote merged branches; merged feat/docs-cleanup and feat/repo-hygiene; reverted feat/minibox-agent-error (LLM API incompatible) and documented design in docs/minibox-agent-design.md; aborted feat/parallel-layer-pulls merge (stale crate path) and documented design in docs/parallel-layer-pulls-design.md; applied pre-session stash (minibox-llm simplification, preflight cleanup, socket rename miniboxd.sock→minibox.sock). [3528b40, 6fbd81d, bc71176, d8809b1, c20ef80]
- 20260423:063311: refactor: binary-consolidation — linuxbox (Linux primitives), minibox facade (re-exports linuxbox+minibox-core), mbx CLI binary (renamed from minibox-cli). Merged worktree branch to main. Expanded xtask gate crate lists to full coverage. [fe62553, 2b2c7e0, 407eda7, d820ade, 6237b24]
