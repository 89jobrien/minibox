# Handoff — minibox (2026-08-17)

**Branch:** develop | **Build:** unknown | **Tests:** unknown
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

## Log

- 20260816.075723: done=145 running=0 pending=19 blocked=0 [44e221e6ec2678f65414b385411377223c5126a5, c3cd3d3e99b78188a593d5bc0dbc7bc979907291, ac2668e242330e916c8dff475a9c5cc097e2051d, 19caf4c7e48f5d6cc4ccc2addda313fc601e83eb, 6cff9986c76385a1c560da1a0fdbfcc408ae5995, 99b6a3119151ea9306eb4732da21ee056b7fa113, af9531927cb860388b5cf3b1ed55a6315c8c63bf, b55c4b27a6a1a536742bf300f9e12eddbf3a97f2, 61e1c706ba2b641885d38fe0fcf5a98d536659ca, 08a72536c5284ad6e86dd90a43fc87289ae18faf]
- 20260816: Completed taskit flow cycle (develop → staging → release → main → develop) with full branch pipeline promotion. Renamed branch pipeline roles to align with taskit convention (develop/staging/release/main). All five commits landed on develop branch with CI green. Workspace builds cleanly; no test failures. [3ed66acc3f8f0fe0f7f39f87b897e1f75c8f1c43, ec59ccea7d17cc0b00a12d59cd39efbe58fac2e1, b482a0aac559c6fb1c49a7b1e28705f9b05d2a34, d5b657d7099d5b8de9a2af26eabdaa67c20f9fb8, 44e221e68c5c7e30ff9c21afef74f03f4ac10b87]
