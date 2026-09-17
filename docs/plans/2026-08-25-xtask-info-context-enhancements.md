# Plan: xtask-info-context enhancements

> **Superseded by V3:** This v2 plan is retained as a historical implementation record.
> The current contract and design are defined in
> [`docs/designs/2026-09-08-accurate-context-snapshot-design.md`](../designs/2026-09-08-accurate-context-snapshot-design.md).
> Do not implement or restore the v2 schema described below.

## 1) Health / state block

- Date: 2026-08-25
- Source A: `godmode context --json`
  - `project`: `minibox`
  - `pending_count`: `6`
  - `critical_path_depth`: `4`
  - `recent_commits`: docs-heavy with latest runtime fix (`81350902`)
- Source B: `cargo xtask info context`
  - `xtask`: `37` source files, `13,034` lines, `171` tests
  - Largest crates by lines: `minibox` (`26,622`), `minibox-core` (`21,093`), `xtask` (`13,034`)

## 2) Impact mapping

- **Primary operator impact**: a fresh agent can derive crate/file/task assignments directly from one command output.
- **Machine-consumer impact**: additive `context_map` block enables deterministic downstream parsing for planning agents.
- **Documentation/schema impact**: CLI schema and docs stay synchronized with emitted snapshot shape.
- **Risk envelope**:
  - Compatibility: preserve v1 fields while bumping `snapshot_version` to `2`.
  - Determinism: explicit ordering for all derived lists.
  - Scope control: only `xtask info context` surface and parity docs/schema updates.

## 3) Provenance + compact summaries

### Provenance

- Runtime/state provenance: `godmode context --json` capture on 2026-08-25.
- Repository/context provenance: `cargo xtask info context` capture on 2026-08-25.
- Planning artifact provenance: this file + `.ctx/godmode/tasks.yaml` entries `xtask-context-1..6`.

### Compact summary

- Add additive `context_map` to context snapshots with three deterministic sections:
  - `crate_assignments`
  - `file_assignments`
  - `task_slices`
- Keep current command shape (`cargo xtask info context [--save]`) and existing fields.
- Validate via focused xtask tests plus `--save` artifact checks.

## 4) Target contract (6-section final structure)

- Command output remains JSON snapshot + optional save artifacts.
- `snapshot_version` becomes `2`.
- New block:
  - `context_map.crate_assignments`: ranked by lines desc, tie-break crate name asc.
  - `context_map.file_assignments`: explicit enhancement surface file list.
  - `context_map.task_slices`: dependency graph with one root, parallel middle, one final verifier.
- No runtime/adapter/daemon behavior changes.

## 5) Actionable phased tasks

### Phase 1 — `xtask-context-1`

**Title**: Add `context_map` schema and red/green snapshot tests  
**Depends on**: none  
**Run**: `cargo test -p xtask context::tests::context_snapshot_includes_context_map`

Actions:

1. Add failing snapshot test for `snapshot_version: 2` and `context_map` keys.
2. Add additive schema types in `xtask/src/context.rs`.
3. Add `context_map` to `ContextSnapshot` and bump version to `2`.

### Phase 2 — `xtask-context-2`

**Title**: Implement deterministic crate assignment derivation  
**Depends on**: `xtask-context-1`  
**Run**: `cargo test -p xtask context::tests::crate_assignments_are_sorted_and_stable`

Actions:

1. Add ordering/tie-break failing test.
2. Implement `derive_crate_assignments` with stable sort and deterministic output.

### Phase 3 — `xtask-context-3`

**Title**: Implement explicit file assignment map for the `info context` enhancement surface  
**Depends on**: `xtask-context-1`  
**Run**: `cargo test -p xtask context::tests::file_assignments_cover_xtask_info_context_surface`

Actions:

1. Add failing test for required file paths.
2. Implement `derive_file_assignments` and emit stable path ordering.

### Phase 4 — `xtask-context-4`

**Title**: Derive task slices with explicit dependencies and parallelizable middle stage  
**Depends on**: `xtask-context-2`, `xtask-context-3`  
**Run**: `cargo test -p xtask context::tests::task_slices_define_expected_dependency_graph`

Actions:

1. Add failing graph test.
2. Implement `derive_task_slices` as `t1 -> {t2,t3} -> t4`.

### Phase 5 — `xtask-context-5`

**Title**: Wire CLI/schema/docs for the enhanced context output contract  
**Depends on**: `xtask-context-4`  
**Run**: `cargo test -p xtask dispatch_args_tests && cargo xtask docs lint`

Actions:

1. Lock dispatch behavior with test-first update.
2. Keep command shape unchanged; update help text only if version wording needs refresh.
3. Update `xtask/schema/cli.schema.json` and `docs/core/XTASK_CLI.mbx.md` for `context_map`.

### Phase 6 — `xtask-context-6`

**Title**: End-to-end verification with saved artifact assertions  
**Depends on**: `xtask-context-5`  
**Run**: `cargo test -p xtask context::tests && cargo xtask info context --save`

Actions:

1. Add save-mode assertion test for `context_map` presence.
2. Verify saved artifacts and required keys in `artifacts/context/snapshot.json`.
3. Run final xtask test gate.

## 6) Validation and consistency checks

- Task IDs and dependencies must remain:
  - `xtask-context-1` -> `xtask-context-2`, `xtask-context-3`
  - `xtask-context-2`, `xtask-context-3` -> `xtask-context-4`
  - `xtask-context-4` -> `xtask-context-5`
  - `xtask-context-5` -> `xtask-context-6`
- `.ctx/godmode/tasks.yaml` titles/runs/dependencies must exactly mirror Phase 1-6 above.
- Scope restriction: only planning artifacts changed by this update.
