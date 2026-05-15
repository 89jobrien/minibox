# Handoff — minibox (2026-05-15)

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
| uncommitted-work | P1 | open | Uncommitted changes (7 files) |

## Log

- 20260515.070756: done=23 running=0 pending=47 blocked=0 [71facaa89fb789d27e6cd6b7d7b195f08d14477d, d50efa9e3a295f9e0647a4650249d783a778881f, 046ba38060b1fa45ea23938d8292d16e5f86422a, 349cae701e7bdbcf386d0788a07a9cbd554f3102, 9f411817eb2022fecae6af96a5edde265cfba22f, 86eda25ad3624e226ee2e12505ef920ec551291f, 449bdf777bd8d4431d86fa22af73ad45dd2432be, 4a879c83e1e8ee45fb465e21a4b1ab97bf73721b, 16e9448c284a230b5f2ff649a547bc788afa02ef, 3fbc743f21b5ab16f6fa74089a04369aa02b637d]
- 20260515.070756: done=23 running=0 pending=47 blocked=0 [71facaa89fb789d27e6cd6b7d7b195f08d14477d, d50efa9e3a295f9e0647a4650249d783a778881f, 046ba38060b1fa45ea23938d8292d16e5f86422a, 349cae701e7bdbcf386d0788a07a9cbd554f3102, 9f411817eb2022fecae6af96a5edde265cfba22f, 86eda25ad3624e226ee2e12505ef920ec551291f, 449bdf777bd8d4431d86fa22af73ad45dd2432be, 4a879c83e1e8ee45fb465e21a4b1ab97bf73721b, 16e9448c284a230b5f2ff649a547bc788afa02ef, 3fbc743f21b5ab16f6fa74089a04369aa02b637d]
- 20260515.065841: done=23 running=0 pending=47 blocked=0 [71facaa89fb789d27e6cd6b7d7b195f08d14477d, d50efa9e3a295f9e0647a4650249d783a778881f, 046ba38060b1fa45ea23938d8292d16e5f86422a, 349cae701e7bdbcf386d0788a07a9cbd554f3102, 9f411817eb2022fecae6af96a5edde265cfba22f, 86eda25ad3624e226ee2e12505ef920ec551291f, 449bdf777bd8d4431d86fa22af73ad45dd2432be, 4a879c83e1e8ee45fb465e21a4b1ab97bf73721b, 16e9448c284a230b5f2ff649a547bc788afa02ef, 3fbc743f21b5ab16f6fa74089a04369aa02b637d]
- 20260515.065841: done=23 running=0 pending=47 blocked=0 [71facaa89fb789d27e6cd6b7d7b195f08d14477d, d50efa9e3a295f9e0647a4650249d783a778881f, 046ba38060b1fa45ea23938d8292d16e5f86422a, 349cae701e7bdbcf386d0788a07a9cbd554f3102, 9f411817eb2022fecae6af96a5edde265cfba22f, 86eda25ad3624e226ee2e12505ef920ec551291f, 449bdf777bd8d4431d86fa22af73ad45dd2432be, 4a879c83e1e8ee45fb465e21a4b1ab97bf73721b, 16e9448c284a230b5f2ff649a547bc788afa02ef, 3fbc743f21b5ab16f6fa74089a04369aa02b637d]
- 20260515.065652: done=23 running=0 pending=47 blocked=0 [71facaa89fb789d27e6cd6b7d7b195f08d14477d, d50efa9e3a295f9e0647a4650249d783a778881f, 046ba38060b1fa45ea23938d8292d16e5f86422a, 349cae701e7bdbcf386d0788a07a9cbd554f3102, 9f411817eb2022fecae6af96a5edde265cfba22f, 86eda25ad3624e226ee2e12505ef920ec551291f, 449bdf777bd8d4431d86fa22af73ad45dd2432be, 4a879c83e1e8ee45fb465e21a4b1ab97bf73721b, 16e9448c284a230b5f2ff649a547bc788afa02ef, 3fbc743f21b5ab16f6fa74089a04369aa02b637d]
