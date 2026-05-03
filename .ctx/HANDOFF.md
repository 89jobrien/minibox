# Handoff — minibox (2026-05-02)

**Branch:** main | **Build:** unknown | **Tests:** unknown

## Items

| ID | P | Status | Title |
|---|---|---|---|
| minibox-33 | P1 | open | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 kernel |
| minibox-46 | P1 | open | feat: PTY/stdio piping for interactive containers (#19) — plan written |
| minibox-55 | P1 | open | feat: minibox owns the full container stack on every OS |
| minibox-crux-plugin | P1 | open | feat: minibox-crux-plugin binary — crux JSON-RPC plugin for minibox adapter |
| minibox-update-upgrade | P1 | open | feat: mbx update + mbx upgrade — restart support stubbed (Wave 3) |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts — OCI layers and bind mounts (#43) / (#66, #75) |
| minibox-52 | P2 | blocked | feat(vz): provision and start minibox-managed Linux VM via Apple VF (#40) / (#76, #84) |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent — in-VM daemon over vsock (#41) / (#78, #88) |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge — stream container stdout/stderr from VM (#42) / (#81, #93) |
| minibox-7 | P2 | blocked | bug(vz): VZ.framework VZErrorInternal(code=1) on macOS 26 ARM64 |

## Log

- 20260502.192909: Session 42: triage-only; confirmed Wave 3 restart (handle_update stop-on-update) and crux-plugin scaffold (9 handlers, 21 tests) are complete; noted pre-existing colima bind mount test failure (169/170 pass, 1 ignored); cargo check clean
- 20260430:143952: Session 39: added conformance tests for macbox/mbx/minibox-macros/winbox; split ci.yml into pr.yml+merge.yml; defaulted adapter to smolvm (krun fallback); documented crux integration state; pruned 8 stale stashes (all pre-rename era); merged next->stable and tagged v0.23.0; synced xtask README with full command inventory; dropped stale minibox-agent-llm-api item. [cb36b64, 17a873d, b596a27, 78d8e11, eee7a61, cdb1ab2, fa6f2e9]
- 20260429:042914: Session 38: landed Wave 2 of update/upgrade (handle_update handler, mbx update CLI, sentinel fixes, CI e2e tests, v0.22.0 bump); added rich bench dashboard (xtask bench rewrite + Chart.js HTML); added compile-time typestate lifecycle module (minibox-core::typestate, 9 tests); explored container ecosystem comparison (tini/conmon/containerd/ctop gaps identified). [9574520, d3459f4, e08a7dc, 277dbf2, 761fd26, 5a4aadf]
- 20260428:124518: Session 37: added mbx update (image refresh) + mbx upgrade (self-update) commands — Wave 1 landed (protocol types, upgrade CLI); Wave 2 agents dispatched (handle_update handler, update CLI). [54df5d0, 1bc83c5]
- 20260428:121625: Session 36: resolved all 6 council recommendations for multi-platform image pull — fixed registry routing bypass (c669ca8), hardened TargetPlatform::parse, fixed proot stderr capture, wired run --platform end-to-end, restored 3 CI jobs, added platform-aware pull handler tests. [c669ca8, f423e19, 49fc9a1, 73703f8, 93b4814, ade65b9, eb6bef2]
