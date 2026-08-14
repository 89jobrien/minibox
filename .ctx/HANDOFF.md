# Handoff — minibox (2026-08-14)

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

- 20260808.184304: done=141 running=0 pending=23 blocked=0 [4fb3661e075a27123d2137892c925be179d57462, a9bf1ad2c850a82ce918aa9f85fd269d30cbd101, cc0f5ec91ca71d01a510d39f162d63373b1396df, 35c3859f1c5a18c549ba798ab73c467dd19df772, 3fd50275b39613bd6ff3e880a7cb9678a0ee48ad, 4cc07fc4abcc2522d270606f54237f59a4d50d77, 3bfe930ae5072ebc5deae2fbf9af0fc5de6c537a, 96ca70dd84b534073fff19142a444b2c5ec9b9fe, 06923bec85615f82f53a0cd137ac14e8a1bbce56, 87f96e57ae2221fcdd0bf4a2f04afac4d7e91a38]
