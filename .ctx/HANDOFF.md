# Handoff — minibox (2026-05-09)

**Branch:** main | **Build:** unknown | **Tests:** unknown

## Items

| ID | P | Status | Title |
|---|---|---|---|
| minibox-33 | P1 | open | feat(winbox): minibox-owned Linux VM on Windows via Hyper-V / WSL2 kernel |
| minibox-46 | P1 | open | feat: PTY/stdio piping for interactive containers (#19) — plan written |
| minibox-55 | P1 | open | feat: minibox owns the full container stack on every OS |
| minibox-crux-plugin | P1 | open | feat: minibox-crux-plugin binary — crux JSON-RPC plugin for minibox adapter |
| minibox-godmode-hook | P1 | open | fix: godmode pre-commit-gate hook CLAUDE_PLUGIN_ROOT not resolving |
| minibox-nightly-geiger | P1 | open | fix(ci): make cargo-geiger non-blocking in nightly workflow |
| minibox-prepush-skip | P1 | open | chore: commit staged prepush skip (gates.rs) — blocked by godmode hook |
| minibox-update-upgrade | P1 | open | feat: mbx update + mbx upgrade — restart support stubbed (Wave 3) |
| minibox-43 | P2 | blocked | feat(vz): virtiofs host-path mounts — OCI layers and bind mounts (#43) / (#66, #75) |
| minibox-52 | P2 | blocked | feat(vz): provision and start minibox-managed Linux VM via Apple VF (#40) / (#76, #84) |
| minibox-53 | P2 | blocked | feat(vz): minibox-agent — in-VM daemon over vsock (#41) / (#78, #88) |
| minibox-54 | P2 | blocked | feat(vz): vsock I/O bridge — stream container stdout/stderr from VM (#42) / (#81, #93) |
| minibox-7 | P2 | blocked | bug(vz): VZ.framework VZErrorInternal(code=1) on macOS 26 ARM64 |

## Log

- 2026-05-09: Session 44: add continue-on-error to geiger job in nightly.yml (minibox-nightly-geiger done); patch godmode hook bazaar cache to absolute path (minibox-godmode-hook done); cargo check clean [f9229da]
- 20260509.225051: Session 43: sentinel autofixer (protocol-drift lock file + NOTE comments); daily nightly/YYYYMMDD release pipeline (nightly.yml tag job, release.yml nightly/* trigger); fixed geiger absolute paths in nightly.yml; skip cargo compilation in pre-commit/pre-push when no Rust files staged; Node.js 24 repo variable; created GH #322 #323 (dead code/unused imports); godmode hook CLAUDE_PLUGIN_ROOT patched in both cache copies (needs session restart) [659f1c2, 7.516e56, 0b93001, f9229da]
- 20260502.192909: Session 42: triage-only; confirmed Wave 3 restart (handle_update stop-on-update) and crux-plugin scaffold (9 handlers, 21 tests) are complete; noted pre-existing colima bind mount test failure (169/170 pass, 1 ignored); cargo check clean
- 20260502.000000: Session 41: added minibox-crux-plugin integration test suite (10 tests via subprocess + mock daemon socket); committed open session-40 changes (crux-plugin scaffold, handler fmt, HANDOFF); morning triage — labeled 14 GitHub issues (p1/p2/p3), wrote daily note [788c10f]
- 20260502.000000: Session 40: implemented Wave 3 restart in handle_update (stop containers on image update); scaffolded minibox-crux-plugin binary (9 handlers, 11 tests, cruxx protocol stdin/stdout loop) [18eb8ea]
