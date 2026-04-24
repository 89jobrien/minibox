# HANDOFF — minibox

**Updated:** 2026-04-24T15:26:52Z | **Branch:** main | **Build:** clean

## Open Items

| ID | Priority | Status | Title |
|----|----------|--------|-------|
| minibox-33 | P1 | open | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 kernel |
| minibox-46 | P1 | open | feat: PTY/stdio piping for interactive containers — plan written |
| minibox-55 | P1 | open | feat: minibox owns the full container stack on every OS |
| minibox-agent-llm-api | P1 | open | Extend minibox-llm with infer() API, then re-land minibox-agent |
| minibox-77 | P1 | open | chore: publish minibox-core and minibox-client to crates.io |
| minibox-78 | P2 | open | feat(ci): add cargo test --all-features job to CI |
| minibox-7 | P2 | blocked | bug(vz): VZ.framework VZErrorInternal(code=1) on macOS 26 ARM64 |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts (#66, #75) |
| minibox-52 | P2 | blocked | feat(vz): provision and start minibox-managed Linux VM via Apple VF |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent — in-VM daemon over vsock |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge — stream container stdout/stderr from VM |
| minibox-65 | P2 | blocked | feat(vz): virtiofs host-path mounts (#75) |
| minibox-66 | P2 | blocked | feat(vz): minibox-agent — in-VM daemon over vsock (#88) |
| minibox-67 | P2 | blocked | feat(vz): vsock I/O bridge (#93) |

## Log (last 5 sessions)

- **26** `2026-04-24` — init_tracing()/MINIBOX_TRACE_LEVEL (e5f178d); audited error system + macros (no gaps); moved dashbox to minibox-plugins (ed1def8); removed stale artifacts; new: publish-core-crates, ci-all-features.
- **25** `2026-04-24` — Closed handler 80% + krun Phase 3. Orca-strait: FEATURE_MATRIX SOT, CI coverage gate, surface tests, parallel-layer-pulls-port.
- **24** `2026-04-23` — Pruned doob backlog 130→100; built full dependency chain.
- **23** `2026-04-23` — Plan audit — 29/30 done confirmed; 10 flagged as LANDED.
- **22** `2026-04-23` — Full test sweep — proptest, conformance, 3 fuzz targets, READMEs for 7 crates.
