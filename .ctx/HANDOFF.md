# minibox handoff

Updated: 20260426:160000 | Branch: main | Build: clean | Tests: passing (macOS gate)

## Open Items

| ID | Priority | Status | Title |
|----|----------|--------|-------|
| minibox-78 | P1 | open | fix(miniboxd): parse_adapter must reject known-but-unavailable adapters |
| minibox-33 | P1 | open | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 kernel |
| minibox-46 | P1 | open | feat: PTY/stdio piping for interactive containers (#19) |
| minibox-55 | P1 | open | feat: minibox owns the full container stack on every OS |
| minibox-77 | P1 | open | chore: consolidate workspace (13->8 crates) and publish to crates.io |
| minibox-agent-llm-api | P1 | blocked | Extend minibox-llm with infer() API, then re-land minibox-agent |
| minibox-7 | P2 | blocked | bug(vz): VZ.framework VZErrorInternal(code=1) on macOS 26 ARM64 |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts |
| minibox-52 | P2 | blocked | feat(vz): provision and start minibox-managed Linux VM |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent in-VM daemon over vsock |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge |

## Recent Log

| Session | Date | Summary |
|---------|------|---------|
| 31 | 20260426:091635 | Phases 3-5 consolidation (13->8 crates). 3 dogfood plans. Test fixes. Council 93%/67%. 10 council todos. |
| 30 | 20260426:095815 | Phases 0-2 consolidation (13->10 crates). Orca-strait agents resolved #95, #120, #134. |
| 29 | 20260426:051901 | 7-phase crate consolidation plan. DEFAULT_ADAPTER_SUITE. smolvm specs. |
| 28 | 20260426:000000 | Closed minibox-78. Pruned 3 duplicate VZ items. |
| 27 | 20260424:192000 | EOD handoff. macbox/krun Phases 1-3 done. |
