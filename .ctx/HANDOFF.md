# Handoff — minibox (2026-08-08)

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
| uncommitted-work | P1 | open | Uncommitted changes (3 files) |

## Log

- 20260808.165619: done=141 running=0 pending=23 blocked=0 [07cc0b6c785277d68d117218993f4871e0e85c77, d66f6692efbe5afc76cb3168f0a353d68c49322d, 0a416f6c1b2740f5abc8eea35a7da3503f8220e2, b6d32e76940c24b85ec13695e2459e5e7cdfa55b, 8642f90bcb7283788b92fd8dff15f5deb85e91fe, 24488417597071ed97d0a60a27f9accd50c5f17d, 2647cdaddeef8236e554b99ce8133cee31739634, 3d56cccac835b0ab21c884a9e05646e6b11f3725, c564a9c033d8c5a452a482454432595987cf69f4, dbc2b2636193daf7cff3bd38273c326cc658283b]
