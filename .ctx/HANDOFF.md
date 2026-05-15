# Handoff — minibox (2026-05-13)

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
| uncommitted-work | P1 | open | Uncommitted changes (28 files) |

## Log

- 20260513.223308: done=23 running=0 pending=1 blocked=0 [1fcb7a3bdc8bba8e249d57c0962ea8f42a0ada13, 2849e2f4b8efd12a3dd60b801706bc583617e85d, 031d8e6b75ade22bb653799dd985b0ebc8a60266, 0ae6a8d8f60902378c8014d708389983a2439c64, 30251c645b2845e3fdaecb874cf4758b7fe57476, 31da360a61aa7d1c5579dba30488b150330b516d, 67fb3d05594ef2d6c55f6f98b1cf274924ef5bc9, 3ea1fa5919bedca098ef2c96dfcbefae099d33c5, e315c8b4032f91cd90e0479fe2a1211808d366a9, 44f46c1772005afc91d4b226128aa1fd4098885d]
- 20260513.095714: done=23 running=0 pending=1 blocked=0 [1fcb7a3bdc8bba8e249d57c0962ea8f42a0ada13, 2849e2f4b8efd12a3dd60b801706bc583617e85d, 031d8e6b75ade22bb653799dd985b0ebc8a60266, 0ae6a8d8f60902378c8014d708389983a2439c64, 30251c645b2845e3fdaecb874cf4758b7fe57476, 31da360a61aa7d1c5579dba30488b150330b516d, 67fb3d05594ef2d6c55f6f98b1cf274924ef5bc9, 3ea1fa5919bedca098ef2c96dfcbefae099d33c5, e315c8b4032f91cd90e0479fe2a1211808d366a9, 44f46c1772005afc91d4b226128aa1fd4098885d]
- 20260513.093854: done=23 running=0 pending=1 blocked=0 [1fcb7a3bdc8bba8e249d57c0962ea8f42a0ada13, 2849e2f4b8efd12a3dd60b801706bc583617e85d, 031d8e6b75ade22bb653799dd985b0ebc8a60266, 0ae6a8d8f60902378c8014d708389983a2439c64, 30251c645b2845e3fdaecb874cf4758b7fe57476, 31da360a61aa7d1c5579dba30488b150330b516d, 67fb3d05594ef2d6c55f6f98b1cf274924ef5bc9, 3ea1fa5919bedca098ef2c96dfcbefae099d33c5, e315c8b4032f91cd90e0479fe2a1211808d366a9, 44f46c1772005afc91d4b226128aa1fd4098885d]
- 20260513.093854: done=23 running=0 pending=1 blocked=0 [1fcb7a3bdc8bba8e249d57c0962ea8f42a0ada13, 2849e2f4b8efd12a3dd60b801706bc583617e85d, 031d8e6b75ade22bb653799dd985b0ebc8a60266, 0ae6a8d8f60902378c8014d708389983a2439c64, 30251c645b2845e3fdaecb874cf4758b7fe57476, 31da360a61aa7d1c5579dba30488b150330b516d, 67fb3d05594ef2d6c55f6f98b1cf274924ef5bc9, 3ea1fa5919bedca098ef2c96dfcbefae099d33c5, e315c8b4032f91cd90e0479fe2a1211808d366a9, 44f46c1772005afc91d4b226128aa1fd4098885d]
- 20260513.093731: done=23 running=0 pending=1 blocked=0 [1fcb7a3bdc8bba8e249d57c0962ea8f42a0ada13, 2849e2f4b8efd12a3dd60b801706bc583617e85d, 031d8e6b75ade22bb653799dd985b0ebc8a60266, 0ae6a8d8f60902378c8014d708389983a2439c64, 30251c645b2845e3fdaecb874cf4758b7fe57476, 31da360a61aa7d1c5579dba30488b150330b516d, 67fb3d05594ef2d6c55f6f98b1cf274924ef5bc9, 3ea1fa5919bedca098ef2c96dfcbefae099d33c5, e315c8b4032f91cd90e0479fe2a1211808d366a9, 44f46c1772005afc91d4b226128aa1fd4098885d]
