# Handoff — minibox (2026-05-14)

**Branch:** develop | **Build:** cargo check passed | **Tests:** cargo test passed
EOD update on branch chore/xtask-borrow-fixtures. Recent 24h work: 6aaf6b3 test(xtask): add borrow fixture verification
3650cad perf(xtask): move release build + conformance from pre-commit to pre-push
3b9901a fix(ci): make cargo-geiger non-blocking in nightly workflow
f9229da perf(xtask): skip cargo compilation in pre-commit when no Rust files staged
0b93001 fix(ci): use absolute paths for cargo-geiger manifest-path
7516e53 feat(ci): add daily nightly/YYYYMMDD tagged releases
659f1c2 fix: apply sentinel suggestion-level fixes
1429d80 fix(ci): pre-trust workspace dir on self-hosted runner before checkout
df5dca0 refactor: move pre-commit bump logic into cargo xtask pre-commit
edbb22e feat(xtask): rate-limit minor version bumps to once per calendar day
c7fd3d3 fix: close clone closure and pipe fds on spawn paths
366af49 fix: update socket-auth regression expectations
8446efa fix(ci): add missing daily_orchestration.rs and fix version bump for workspace deps
8debe13 fix(ci): harden release, reviewdog, and issue triage workflows
10e4a6c fix(ci): ignore pty_exec_echo_roundtrip in unprivileged CI
5a3568b ci: retrigger merge workflow after stuck run
60bb538 ci: retrigger merge workflow
269947e refactor: move xtask crate to workspace root
5681a39 fix(ci): replace cancel-on-failure with ci-ok sentinel job
d4fb4b7 ci: trigger workflow
89d071a ci(merge): add lint + unit test jobs on ubuntu-latest for all pushes
9dfbf82 ci: migrate macOS CI from self-hosted to GitHub runners
327b559 drop(vm): remove QEMU vm_image and vm_run xtask commands
00ee442 drop(vz): remove VZ adapter and all associated code
30bfa14 Fix typo in README.md regarding software structure. Validation: cargo check passed; cargo test passed.

## Items

| ID | P | Status | Title |
|---|---|---|---|
| uncommitted-work | P1 | open | Uncommitted changes (6 files) |

## Log

- 20260515.034950: done=23 running=0 pending=47 blocked=0 [dbab202f262f89f814e8bb1632b224b6e4e18a37, cd5b336f0870029818a8f30d20ce7c95c8b3a2f9, a8f633ce818066a44b524b6cd0262e51526eed78, 721ecd5e396d5a5b4ca66e821a86cc0b05850bf7, 27a0f641c5e3b6928f312f1a18469b2cdacc17e9, 1f302515954efdd44d32ab98807e77e828e945fc, 2b0c5beec4765c87b690ee32e45c06f829baa314, 583fe5b09772167a93752e899f4d669547349e63, 61489c81279d28451a2b06f9f80184a2f6158431, 1440aa82c0537b2c03313a2cf8272b9a6f0a2482]
- 20260515.034950: done=23 running=0 pending=47 blocked=0 [dbab202f262f89f814e8bb1632b224b6e4e18a37, cd5b336f0870029818a8f30d20ce7c95c8b3a2f9, a8f633ce818066a44b524b6cd0262e51526eed78, 721ecd5e396d5a5b4ca66e821a86cc0b05850bf7, 27a0f641c5e3b6928f312f1a18469b2cdacc17e9, 1f302515954efdd44d32ab98807e77e828e945fc, 2b0c5beec4765c87b690ee32e45c06f829baa314, 583fe5b09772167a93752e899f4d669547349e63, 61489c81279d28451a2b06f9f80184a2f6158431, 1440aa82c0537b2c03313a2cf8272b9a6f0a2482]
- 20260515.034504: done=23 running=0 pending=47 blocked=0 [dbab202f262f89f814e8bb1632b224b6e4e18a37, cd5b336f0870029818a8f30d20ce7c95c8b3a2f9, a8f633ce818066a44b524b6cd0262e51526eed78, 721ecd5e396d5a5b4ca66e821a86cc0b05850bf7, 27a0f641c5e3b6928f312f1a18469b2cdacc17e9, 1f302515954efdd44d32ab98807e77e828e945fc, 2b0c5beec4765c87b690ee32e45c06f829baa314, 583fe5b09772167a93752e899f4d669547349e63, 61489c81279d28451a2b06f9f80184a2f6158431, 1440aa82c0537b2c03313a2cf8272b9a6f0a2482]
- 20260515.034504: done=23 running=0 pending=47 blocked=0 [dbab202f262f89f814e8bb1632b224b6e4e18a37, cd5b336f0870029818a8f30d20ce7c95c8b3a2f9, a8f633ce818066a44b524b6cd0262e51526eed78, 721ecd5e396d5a5b4ca66e821a86cc0b05850bf7, 27a0f641c5e3b6928f312f1a18469b2cdacc17e9, 1f302515954efdd44d32ab98807e77e828e945fc, 2b0c5beec4765c87b690ee32e45c06f829baa314, 583fe5b09772167a93752e899f4d669547349e63, 61489c81279d28451a2b06f9f80184a2f6158431, 1440aa82c0537b2c03313a2cf8272b9a6f0a2482]
- 20260515.033503: done=23 running=0 pending=47 blocked=0 [dbab202f262f89f814e8bb1632b224b6e4e18a37, cd5b336f0870029818a8f30d20ce7c95c8b3a2f9, a8f633ce818066a44b524b6cd0262e51526eed78, 721ecd5e396d5a5b4ca66e821a86cc0b05850bf7, 27a0f641c5e3b6928f312f1a18469b2cdacc17e9, 1f302515954efdd44d32ab98807e77e828e945fc, 2b0c5beec4765c87b690ee32e45c06f829baa314, 583fe5b09772167a93752e899f4d669547349e63, 61489c81279d28451a2b06f9f80184a2f6158431, 1440aa82c0537b2c03313a2cf8272b9a6f0a2482]
