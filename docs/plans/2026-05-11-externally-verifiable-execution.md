# Lightweight Externally Verifiable Execution

> **For agentic workers:** Run `/godmode:tackle-issues` to dispatch parallel subagents per task group. Use `/godmode:test-driven-development` for each implementation step. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add manifest-first externally verifiable execution to minibox by persisting deterministic run manifests and workload digests before adding policy enforcement and CLI verification.

**Architecture:** `minibox-core` owns portable manifest and policy domain types, `minibox` daemon run handling builds and persists manifests before spawn, `miniboxd` wires policy configuration into all adapter suites, and `mbx` exposes manifest inspection and verification through protocol requests.

**Tech Stack:** Rust 2024 workspace, `serde`, `serde_json`, `sha2`, `hex`, existing OCI image metadata, existing daemon JSON protocol, existing `just` and `cargo xtask` gates.

**Repo:** `/Users/joe/dev/minibox`

**GitHub issue:** null

---

## Web Validation

The design aligns with current supply-chain and container standards:

- OCI image descriptors identify content with `mediaType`, `digest`, and `size`; descriptor consumers should verify retrieved content against the digest, and SHA-256 support is required by the OCI image spec.
- OCI image manifests are already content-addressable and reference config and layers by descriptors, which validates using existing manifest/config/layer digests as the image portion of workload identity.
- in-toto Statement v1 binds metadata to immutable subjects via digest and `predicateType`, which validates designing minibox manifests so they can later be wrapped as in-toto statements.
- SLSA provenance models externally controlled inputs as `externalParameters` and resolved immutable artifacts as `resolvedDependencies`, which validates separating run request fields from image/layer digests in the manifest.
- SLSA verification guidance says consumers should compare provenance against expected values and reject unexpected external parameters, which validates an explicit measured-input policy layer.
- Sigstore Cosign supports signing arbitrary predicate files and validating in-toto attestations against CUE/Rego policy, which validates leaving a future signing/export path while implementing local manifests first.

---

## Task 1: Define execution manifest domain model

**Files:**

- `crates/minibox-core/src/domain.rs`
- `crates/minibox-core/src/lib.rs`
- `crates/minibox-core/tests/execution_manifest.rs`

- [ ] **Step 1:** Add `ExecutionManifest`, `ExecutionManifestSubject`, `ExecutionManifestImage`, `ExecutionManifestRuntime`, `ExecutionManifestRequest`, `ExecutionManifestMount`, `ExecutionManifestResourceLimits`, and `ExecutionManifestDigest` domain structs in `minibox-core`.
- [ ] **Step 2:** Add deterministic workload digest calculation that hashes a stable JSON projection excluding volatile fields such as creation timestamp, manifest file path, and the digest field itself.
- [ ] **Step 3:** Represent secret-bearing environment data as variable names plus SHA-256 value digests, never plaintext values.
- [ ] **Step 4:** Add unit tests proving equal semantic input produces equal workload digest, changed command/env/mount/network/image digest changes the workload digest, and volatile fields do not affect it.
- [ ] **Commit:** `git commit -m "feat(core): add execution manifest model"`

---

## Task 2: Refactor shared run preparation

**Files:**

- `crates/minibox/src/daemon/handler.rs`
- `crates/minibox/tests/daemon_handler_tests.rs`

- [ ] **Step 1:** Extract common run setup from `run_inner` and `run_inner_capture` into a shared helper that accepts `capture_output` and returns prepared run state.
- [ ] **Step 2:** Keep image resolution, platform registry selection, image pull/cache check, layer lookup, rootfs setup, cgroup setup, network setup, `ContainerRecord` construction, and `ContainerSpawnConfig` construction in the shared helper.
- [ ] **Step 3:** Preserve existing behavior for non-streaming runs and ephemeral streaming runs, including first streaming response, output capture, state transitions, and auto-remove behavior.
- [ ] **Step 4:** Add regression tests for policy rejection, duplicate name rejection, non-streaming success, and ephemeral setup using the shared preparation path.
- [ ] **Commit:** `git commit -m "refactor(daemon): share run preparation path"`

---

## Task 3: Persist execution manifests before spawn

**Files:**

- `crates/minibox/src/daemon/handler.rs`
- `crates/minibox/src/daemon/state.rs`
- `crates/minibox/tests/daemon_handler_tests.rs`
- `docs/SECURITY_INVARIANTS.mbx.md`

- [ ] **Step 1:** Build an `ExecutionManifest` in the shared run preparation helper after image/rootfs/resource/network inputs are known and before `spawn_process`.
- [ ] **Step 2:** Persist the manifest to `{containers_base}/{id}/execution-manifest.json` with owner-only permissions on Unix.
- [ ] **Step 3:** Add `execution_manifest_path` and `workload_digest` fields to `ContainerRecord` with serde defaults for backward compatibility.
- [ ] **Step 4:** Fail closed if manifest serialization or persistence fails before spawn, and clean up partially created state where practical.
- [ ] **Step 5:** Add tracing fields for `container_id` and `workload_digest` around successful manifest creation and runtime spawn.
- [ ] **Step 6:** Document the new invariant that every successful run must have a persisted pre-spawn manifest.
- [ ] **Commit:** `git commit -m "feat(daemon): persist execution manifests"`

---

## Task 4: Add measured execution policy

**Files:**

- `crates/minibox-core/src/domain.rs`
- `crates/minibox/src/daemon/handler.rs`
- `crates/miniboxd/src/main.rs`
- `crates/minibox/tests/daemon_handler_tests.rs`
- `crates/miniboxd/tests/conformance_wiring.rs`

- [ ] **Step 1:** Add `ExecutionPolicy` and `ExecutionPolicyDecision` types supporting allowed image refs, allowed image digests, required platform, allowed network modes, bind mount allowance, privileged allowance, and expected workload digest.
- [ ] **Step 2:** Add policy evaluation against `ExecutionManifest` with fail-closed errors for unknown or mismatched measured inputs.
- [ ] **Step 3:** Wire optional policy loading into `HandlerDependencies` and all miniboxd adapter suite builders, initially from `MINIBOX_EXECUTION_POLICY`.
- [ ] **Step 4:** Run the existing cheap `ContainerPolicy` gate before image work, then run measured `ExecutionPolicy` after manifest construction and before spawn.
- [ ] **Step 5:** Add tests for denied privileged mode, denied host network, image ref mismatch, image digest mismatch, workload digest mismatch, and allowed policy pass.
- [ ] **Commit:** `git commit -m "feat(daemon): enforce measured execution policy"`

---

## Task 5: Expose manifest inspection and verification

**Files:**

- `crates/minibox-core/src/protocol.rs`
- `crates/mbx/src/main.rs`
- `crates/mbx/src/commands/manifest.rs`
- `crates/minibox/src/daemon/handler.rs`
- `crates/minibox/tests/daemon_handler_tests.rs`
- `crates/mbx/tests/cli_subprocess.rs`

- [ ] **Step 1:** Add protocol requests and responses for reading a container execution manifest and verifying it against a policy file.
- [ ] **Step 2:** Implement daemon handlers that read the persisted manifest by container ID or unambiguous prefix and return JSON or a structured verification result.
- [ ] **Step 3:** Add `mbx manifest <id>` to print the stored execution manifest without changing `mbx run` streaming behavior.
- [ ] **Step 4:** Add `mbx verify <id> --policy <file>` to evaluate a persisted manifest against a local policy document.
- [ ] **Step 5:** Add protocol compatibility tests and CLI subprocess tests for manifest output and verification failure messaging.
- [ ] **Commit:** `git commit -m "feat(cli): inspect and verify execution manifests"`

---

## Task 6: Document format and future attestation path

**Files:**

- `docs/ARCHITECTURE.mbx.md`
- `docs/SECURITY_INVARIANTS.mbx.md`
- `docs/superpowers/plans/2026-05-11-externally-verifiable-execution.md`
- `README.md`

- [ ] **Step 1:** Document the `ExecutionManifest` schema, workload digest inputs, and which fields are intentionally excluded from the digest.
- [ ] **Step 2:** Document the policy file shape and fail-closed behavior.
- [ ] **Step 3:** Document explicit non-goals for this phase: no TEE attestation, no encrypted attested sessions, no transparency log, and no signing requirement.
- [ ] **Step 4:** Add a future path for wrapping the manifest as an in-toto statement and signing/exporting it with Cosign or another signing tool.
- [ ] **Step 5:** Update user-facing docs with basic examples for manifest inspection and policy verification.
- [ ] **Commit:** `git commit -m "docs: describe verifiable execution manifests"`

---

## Verification Checklist

- [ ] All tasks complete
- [ ] `cargo fmt --all --check` passes
- [ ] `cargo clippy -p minibox -p minibox-macros -p mbx -p macbox -p miniboxd -- -D warnings` passes
- [ ] `cargo xtask test-unit` passes
- [ ] New manifest digest tests pass
- [ ] New policy evaluation tests pass
- [ ] New protocol and CLI tests pass
- [ ] Existing streaming `mbx run` behavior remains unchanged
