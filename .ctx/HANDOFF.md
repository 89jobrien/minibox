# Handoff — minibox (2026-04-17)

**Branch:** main | **Build:** ok | **Tests:** passing (cargo xtask test-unit)

## Items

| ID | P | Status | Title |
|---|---|---|---|
| minibox-26 | P2 | open | bug(exec): fork() inside active Tokio runtime (#60) |
| minibox-33 | P2 | open | feat(winbox): minibox-owned Linux VM via Hyper-V/WSL2 (#45) |
| minibox-34 | P2 | open | Tier 2 mbx-dagu fixes (#31, #35, #36) |
| minibox-46 | P2 | open | feat: PTY/stdio piping plan written (#70) |
| minibox-55 | P2 | open | feat: minibox owns full container stack on every OS (#87) |
| minibox-56 | P2 | open | feat: container networking — veth/bridge (#20) |
| minibox-58 | P2 | open | bug: no CI gate on release version bumps (#55) |
| minibox-68 | P2 | open | feat(xtask): VM image rootfs overlay directory |
| minibox-69 | P2 | open | fix: migrate ContainerState::Paused to daemonbox (#104) |
| minibox-102 | P2 | open | refactor(daemonbox): extract StateRepository port (#102) — ready to start |
| minibox-101 | P2 | open | refactor(daemonbox): decompose HandlerDependencies god object (#101) — after #102 |
| minibox-100 | P2 | open | refactor(domain): replace tokio channel params in port sigs (#100) — after #101 |
| minibox-105 | P2 | open | refactor(domain): split FilesystemProvider trait (#105) — after #100 |
| minibox-106 | P2 | open | refactor(domain): remove Colima variant from BackendRootfsMetadata (#106) — after #105 |
| minibox-7  | P2 | blocked | VZ.framework VZErrorInternal Apple bug |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts (#66/#75) |
| minibox-52 | P2 | blocked | feat(vz): VZ VM provisioning (#76/#84) |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent over vsock (#78/#88) |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge (#81/#93) |
| minibox-65 | P2 | blocked | feat(vz): virtiofs host-path mounts (#75) |
| minibox-66 | P2 | blocked | feat(vz): minibox-agent (#88) |
| minibox-67 | P2 | blocked | feat(vz): vsock I/O bridge (#93) |

## T-arch Chain Status

#103 done → **#102 ready** → #101 → #100 → #105 → #106

#103 (RegistryRouter port): committed 2681835, test follow-up 9b9e9ce, issue closed.
#102 (StateRepository port): planned, no branch yet. Start here next session.

## Log

- 2026-04-17 (this session): T-arch triage + label sprint — created 7 track labels, applied
  to all 44 open issues; closed T-bugfix (#52/#55/#56/#60) and T-colima (#86/#89/#90/#35/#36)
  as already implemented; wired #68 (test-unit conformance gate); #103 (RegistryRouter port)
  committed by forge agent [2681835] + test follow-up committed [9b9e9ce], issue closed.
  T-arch chain next: #102.
- 2026-04-17: Test coverage sprint closed minibox-18/30/31/32/37/38/62/63/64 — escape
  detection (4 tests), GKE adapter suite (18), lifecycle failures (7), Colima conformance
  (5); minibox-oci crate extracted; Paused OCP fix partial (minibox-69 open).
  [a4ccf4c, b3c8b5f, 92578fd]
- 2026-04-16: Pruned 10 duplicate HANDOFF items; installed QEMU; designed VM overlay
  (minibox-68); P1 sprint closed minibox-40/41/42/49/50/51/57/59/60. [6d6e6d8]
- 2026-04-14: Fixed minibox-25 (cargo build --locked CI) and minibox-27 (e2e gate).
- 2026-04-11: Conformance suite Phases 1–3 shipped. [851ca3e..3cb4b07]
- 2026-04-09: CI recovery; PTY/stdio piping shipped. [89664da, a02f229..c4a0044]
