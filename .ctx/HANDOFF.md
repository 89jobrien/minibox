# Handoff — minibox (2026-05-12)

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
| uncommitted-work | P1 | open | Uncommitted changes (11 files) |

## Log

- 20260512.144332: done=23 running=0 pending=1 blocked=0 [3ea1fa5919bedca098ef2c96dfcbefae099d33c5, e315c8b4032f91cd90e0479fe2a1211808d366a9, 44f46c1772005afc91d4b226128aa1fd4098885d, 847596f164c78e2a4095aac9be0ef2cc7ef6781e, c4846690ab738a4134a367320ff09f148605a407, 6f3d45c2ced3ab4fde42be8630166485ec49b87f, 36eaf1d5adc9e28000a91c2f93159d0e7aa07f45, 5a69cf494fbc7d658afee17a414822e8c2d208b0, a2e53c48d74496c208dfbb0b98b424360b50f28c, 8e9e2c1436d5b6c67f1977e195db55bb9029788d]
- 20260512.144332: done=23 running=0 pending=1 blocked=0 [3ea1fa5919bedca098ef2c96dfcbefae099d33c5, e315c8b4032f91cd90e0479fe2a1211808d366a9, 44f46c1772005afc91d4b226128aa1fd4098885d, 847596f164c78e2a4095aac9be0ef2cc7ef6781e, c4846690ab738a4134a367320ff09f148605a407, 6f3d45c2ced3ab4fde42be8630166485ec49b87f, 36eaf1d5adc9e28000a91c2f93159d0e7aa07f45, 5a69cf494fbc7d658afee17a414822e8c2d208b0, a2e53c48d74496c208dfbb0b98b424360b50f28c, 8e9e2c1436d5b6c67f1977e195db55bb9029788d]
- 20260512.135039: done=23 running=0 pending=1 blocked=0 [e315c8b4032f91cd90e0479fe2a1211808d366a9, 44f46c1772005afc91d4b226128aa1fd4098885d, 847596f164c78e2a4095aac9be0ef2cc7ef6781e, c4846690ab738a4134a367320ff09f148605a407, 6f3d45c2ced3ab4fde42be8630166485ec49b87f, 36eaf1d5adc9e28000a91c2f93159d0e7aa07f45, 5a69cf494fbc7d658afee17a414822e8c2d208b0, a2e53c48d74496c208dfbb0b98b424360b50f28c, 8e9e2c1436d5b6c67f1977e195db55bb9029788d, 140b833456025ed2910d23690bd3bb4a0428d401]
- 20260512.135039: done=23 running=0 pending=1 blocked=0 [e315c8b4032f91cd90e0479fe2a1211808d366a9, 44f46c1772005afc91d4b226128aa1fd4098885d, 847596f164c78e2a4095aac9be0ef2cc7ef6781e, c4846690ab738a4134a367320ff09f148605a407, 6f3d45c2ced3ab4fde42be8630166485ec49b87f, 36eaf1d5adc9e28000a91c2f93159d0e7aa07f45, 5a69cf494fbc7d658afee17a414822e8c2d208b0, a2e53c48d74496c208dfbb0b98b424360b50f28c, 8e9e2c1436d5b6c67f1977e195db55bb9029788d, 140b833456025ed2910d23690bd3bb4a0428d401]
- 20260512.134758: done=23 running=0 pending=1 blocked=0 [e315c8b4032f91cd90e0479fe2a1211808d366a9, 44f46c1772005afc91d4b226128aa1fd4098885d, 847596f164c78e2a4095aac9be0ef2cc7ef6781e, c4846690ab738a4134a367320ff09f148605a407, 6f3d45c2ced3ab4fde42be8630166485ec49b87f, 36eaf1d5adc9e28000a91c2f93159d0e7aa07f45, 5a69cf494fbc7d658afee17a414822e8c2d208b0, a2e53c48d74496c208dfbb0b98b424360b50f28c, 8e9e2c1436d5b6c67f1977e195db55bb9029788d, 140b833456025ed2910d23690bd3bb4a0428d401]
