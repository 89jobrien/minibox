# Handoff — minibox (2026-05-11)

**Branch:** main | **Build:** cargo check passed | **Tests:** cargo test passed
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
| crux-plugin-2 | P2 | blocked | Add mounts field wiring to minibox::container::run handler |

## Log

- 20260511.234919: done=3 running=0 pending=20 blocked=1 [f4673b51e637dffee009596bfd107ed19398bde7, c8417068f915857068dc7b4b244a2cdbec067ffe, 23002e376dd70f3baec1c7565cd87eee70282c2a, f8359b1e451e22b8cefc6d6f5851fc2703987712, 03b221d2047328a895b431b72cd6665c8d45e7e6, ae28db1d3a18f6a85c5dbd0d8cf9e4d56485d29b, fb625f69f5ea9a1e74be5d78640a1d4ed29e63b6, 6797004564e90a0c3643f90ea0994b225530f581, b9a9650feae1890391f67378f3490c9752304d12, a8ed9dc89a9cc0d94be0ab7a3888d40add12b2e0]
- 20260511.234919: done=3 running=0 pending=20 blocked=1 [f4673b51e637dffee009596bfd107ed19398bde7, c8417068f915857068dc7b4b244a2cdbec067ffe, 23002e376dd70f3baec1c7565cd87eee70282c2a, f8359b1e451e22b8cefc6d6f5851fc2703987712, 03b221d2047328a895b431b72cd6665c8d45e7e6, ae28db1d3a18f6a85c5dbd0d8cf9e4d56485d29b, fb625f69f5ea9a1e74be5d78640a1d4ed29e63b6, 6797004564e90a0c3643f90ea0994b225530f581, b9a9650feae1890391f67378f3490c9752304d12, a8ed9dc89a9cc0d94be0ab7a3888d40add12b2e0]
- 20260511.234848: done=3 running=0 pending=20 blocked=1 [f4673b51e637dffee009596bfd107ed19398bde7, c8417068f915857068dc7b4b244a2cdbec067ffe, 23002e376dd70f3baec1c7565cd87eee70282c2a, f8359b1e451e22b8cefc6d6f5851fc2703987712, 03b221d2047328a895b431b72cd6665c8d45e7e6, ae28db1d3a18f6a85c5dbd0d8cf9e4d56485d29b, fb625f69f5ea9a1e74be5d78640a1d4ed29e63b6, 6797004564e90a0c3643f90ea0994b225530f581, b9a9650feae1890391f67378f3490c9752304d12, a8ed9dc89a9cc0d94be0ab7a3888d40add12b2e0]
- 20260511.234546: done=3 running=0 pending=20 blocked=1 [f4673b51e637dffee009596bfd107ed19398bde7, c8417068f915857068dc7b4b244a2cdbec067ffe, 23002e376dd70f3baec1c7565cd87eee70282c2a, f8359b1e451e22b8cefc6d6f5851fc2703987712, 03b221d2047328a895b431b72cd6665c8d45e7e6, ae28db1d3a18f6a85c5dbd0d8cf9e4d56485d29b, fb625f69f5ea9a1e74be5d78640a1d4ed29e63b6, 6797004564e90a0c3643f90ea0994b225530f581, b9a9650feae1890391f67378f3490c9752304d12, a8ed9dc89a9cc0d94be0ab7a3888d40add12b2e0]
- 20260511.234546: done=3 running=0 pending=20 blocked=1 [f4673b51e637dffee009596bfd107ed19398bde7, c8417068f915857068dc7b4b244a2cdbec067ffe, 23002e376dd70f3baec1c7565cd87eee70282c2a, f8359b1e451e22b8cefc6d6f5851fc2703987712, 03b221d2047328a895b431b72cd6665c8d45e7e6, ae28db1d3a18f6a85c5dbd0d8cf9e4d56485d29b, fb625f69f5ea9a1e74be5d78640a1d4ed29e63b6, 6797004564e90a0c3643f90ea0994b225530f581, b9a9650feae1890391f67378f3490c9752304d12, a8ed9dc89a9cc0d94be0ab7a3888d40add12b2e0]
