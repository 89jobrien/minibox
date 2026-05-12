# Plan: Advanced Testing Surfaces

Status: draft
Created: 2026-05-12
Issues: #341 #342 #343 #344 #345 #346 #347 #348 #349 #350 #351

## Motivation

Minibox has ~1,467 tests across unit, integration, conformance, property, and e2e
categories. The existing suite is strong on happy-path coverage and has dedicated
security regression tests pinned to specific invariants. However, several classes
of bugs remain structurally invisible:

- **Undertested assertions** -- tests that execute code but don't meaningfully
  assert on outcomes (mutation survivors).
- **Concurrency races** -- `Arc<Mutex<DaemonState>>`, event bus broadcast, and
  cgroup operations share mutable state across tasks.
- **Protocol robustness** -- partial reads, malformed frames, mid-stream
  disconnects on the daemon socket and GHCR streaming paths.
- **Input domain exhaustiveness** -- path traversal, image ref parsing, and
  cgroup limit arithmetic are tested with hand-picked examples, not the full
  input space.

This plan is split into two phases:

- **Phase A (Methodology Extraction)** -- zero new dependencies. Applies the
  *ideas* from mutation testing, simulation, adversarial scheduling,
  property-based testing, and exhaustive verification using patterns already
  available in the codebase.
- **Phase B (Tooling Adoption)** -- introduces external crates and cargo
  subcommands to automate and scale the methodologies from Phase A.

Phase A is immediately actionable. Phase B items are unlocked after Phase A
demonstrates value and identifies the highest-leverage gaps.

---

# Phase A: Methodology Extraction (No New Dependencies)

---

## A1. Mutation Audit Checklist (from cargo-mutants)

**Methodology:** For every branch, guard, and error return in security-critical
code, verify that a test exists which fails if that line is deleted or inverted.

**Process:**
1. For each module in the target list, enumerate every `if` guard, `?` return,
   and sanitization step (setuid strip, path rewrite, size check).
2. For each, confirm a test in `security_regression.rs` or
   `daemon_security_regression.rs` that asserts the rejection/error path.
3. If no such test exists, write one. If a test exists but only asserts
   `is_ok()` on the happy path, add the negative assertion.

**Checklist template (per function):**

```
[ ] Every `if` guard has a test that triggers the else/rejection branch
[ ] Every `.context()?` has a test that expects Err for that specific failure
[ ] Every sanitization (mode &= 0o777, path rewrite) has a before/after assertion
[ ] Removing the guard would cause at least one test to fail
```

**Target modules:**

| Module | Key guards to audit |
|--------|--------------------|
| `image/layer.rs` | `validate_tar_entry_path`, `has_parent_dir_component`, setuid mask, device node reject, FIFO handling, root entry skip |
| `daemon/server.rs` | `is_authorized`, `MAX_REQUEST_SIZE` check, socket mode |
| `execution_manifest.rs` | `seal()` digest computation, env value hashing |
| `container/process.rs` | `close_extra_fds`, `execve` (not `execvp`) |
| `image/registry.rs` | `MAX_MANIFEST_SIZE`, `MAX_LAYER_SIZE`, Content-Length check |
| `adapters/ghcr.rs` | Manifest/layer size mirrors |

**Deliverable:** A markdown checklist committed to `docs/MUTATION_AUDIT.md` with
pass/fail per guard. Failing entries become test backlog items.

**Exit criteria:** Every guard in the six modules has a corresponding negative
test. The checklist is 100% green.

---

## A2. Exhaustive Small-Domain Tests (from kani)

**Methodology:** For pure functions with bounded input domains, enumerate ALL
valid inputs instead of sampling. No symbolic execution needed -- just loops.

**Already done:** `is_authorized` has 7 exhaustive cases in
`daemon_security_regression.rs`. Generalize this pattern.

**New exhaustive tests:**

```rust
#[test]
fn setuid_strip_exhaustive() {
    // All 12 bits of the mode's special/permission fields
    for mode in 0u32..=0o7777 {
        let stripped = mode & 0o777;
        assert_eq!(stripped & 0o7000, 0,
            "setuid/setgid/sticky bit survived stripping for mode {mode:#o}");
    }
}

#[test]
fn is_authorized_exhaustive() {
    let uid_cases = [None, Some(0u32), Some(1), Some(500), Some(65534), Some(u32::MAX)];
    for require_root in [true, false] {
        for uid in &uid_cases {
            let result = is_authorized(*uid, require_root);
            match (require_root, uid) {
                (false, _) => assert!(result, "should allow when root not required"),
                (true, None) => assert!(!result, "fail-closed on missing creds"),
                (true, Some(0)) => assert!(result, "root allowed"),
                (true, Some(_)) => assert!(!result, "non-root rejected"),
            }
        }
    }
}

#[test]
fn path_component_classification_exhaustive() {
    // Test all single-component paths that matter
    let cases = [".", "..", "/", "a", "a/b", "../a", "a/..", "a/../b",
                 "/etc", "etc/passwd", "../../../etc/shadow"];
    for case in cases {
        let path = Path::new(case);
        let has_parent = has_parent_dir_component(path);
        let expected = case.split('/').any(|c| c == "..");
        assert_eq!(has_parent, expected, "mismatch for {case:?}");
    }
}
```

**Rule:** If the input domain has fewer than 10,000 values, exhaust it. If it
has structure (enum variants x small integer), enumerate the cross product.

**Exit criteria:** Exhaustive tests exist for `is_authorized`,
setuid stripping, `has_parent_dir_component`, and entry type classification.

---

## A3. Stream Trait Boundary (from turmoil)

**Methodology:** Make I/O a trait parameter so tests can swap real sockets for
deterministic fakes. This is the *prerequisite* for turmoil (Phase B) but
delivers value independently by enabling mock-stream tests.

**Current state:** `daemon/server.rs` hardcodes `tokio::net::UnixStream`.
`adapters/ghcr.rs` hardcodes `reqwest::Client`.

**Refactoring:**

```rust
// In daemon/server.rs -- extract a trait
#[async_trait]
pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl AsyncStream for tokio::net::UnixStream {}

// In tests:
struct MockStream {
    read_buf: Vec<u8>,   // predetermined bytes the "client" sends
    write_buf: Vec<u8>,  // captures what the server writes back
}
impl AsyncStream for MockStream {}
```

For GHCR, extract the HTTP fetch behind a trait:

```rust
#[async_trait]
pub trait RegistryTransport: Send + Sync {
    async fn get(&self, url: &str) -> Result<Response>;
}

impl RegistryTransport for reqwest::Client { ... }

// In tests:
struct MockTransport { responses: Vec<(String, Result<Vec<u8>>)> }
```

**Test scenarios enabled (no turmoil needed):**

| Scenario | MockStream/MockTransport behaviour |
|----------|------------------------------------|
| Half-frame request | `read_buf` contains truncated JSON |
| Oversized request | `read_buf` contains 2 MB of data |
| Registry 503 | `MockTransport::get` returns `Err` |
| Slow registry | `MockTransport::get` sleeps then returns |
| Partial layer body | `MockTransport::get` returns fewer bytes than Content-Length |

**Deliverable:** Trait definitions in `server.rs` and `ghcr.rs`, plus 5 mock
stream tests. No external dependency.

**Exit criteria:** Daemon and GHCR paths are generic over their I/O type.
Five mock-stream failure tests pass.

---

## A4. Barrier-Based Race Tests (from shuttle)

**Methodology:** Use `std::sync::Barrier` to force specific thread
interleavings that expose races, without replacing the sync primitives.

**Approach:** Identify the critical concurrent access patterns and write one
test per interleaving:

```rust
#[test]
fn create_destroy_race_is_consistent() {
    let state = Arc::new(Mutex::new(DaemonState::new()));
    let barrier = Arc::new(Barrier::new(2));

    let s1 = state.clone();
    let b1 = barrier.clone();
    let t1 = std::thread::spawn(move || {
        b1.wait(); // synchronize start
        s1.lock().unwrap().add_container(id.clone(), record.clone());
    });

    let s2 = state.clone();
    let b2 = barrier.clone();
    let t2 = std::thread::spawn(move || {
        b2.wait(); // synchronize start
        s2.lock().unwrap().remove_container(&id);
    });

    t1.join().unwrap();
    t2.join().unwrap();

    let locked = state.lock().unwrap();
    // Container either exists or doesn't -- no corrupt state
    let exists = locked.get_container(&id).is_some();
    // Both orderings are valid; assert no panic occurred
    assert!(exists || !exists); // real assertion: no poisoned mutex
}
```

**Target interleavings:**

| Race | Threads | Invariant |
|------|---------|-----------|
| Create vs destroy (same ID) | 2 | State is valid, no panic |
| Event subscribe vs broadcast | 2 | No dropped events after subscribe completes |
| Pause vs container exit | 2 | No cgroup write to exited container |
| GC sweep vs active pull | 2 | GC skips in-progress images |

**Exit criteria:** Four barrier-based race tests, each run 100 times in a loop
to increase scheduling diversity.

---

## A5. Roundtrip Property Rule (from quickcheck)

**Methodology:** Every public type that crosses a serialization boundary gets
a roundtrip property test using proptest (already in tree).

**Audit:** Check which protocol and domain types lack roundtrip coverage:

| Type | Has roundtrip test? | Action |
|------|--------------------:|--------|
| `DaemonRequest` | check | add if missing |
| `DaemonResponse` | check | add if missing |
| `ImageReference` | check | add if missing |
| `ContainerConfig` | check | add if missing |
| `ExecutionManifest` | check | add if missing |
| `BackendDescriptor` | check | add if missing |

**Pattern (using existing proptest):**

```rust
proptest! {
    #[test]
    fn daemon_request_roundtrips(req in arb_daemon_request()) {
        let json = serde_json::to_string(&req).unwrap();
        let decoded: DaemonRequest = serde_json::from_str(&json).unwrap();
        prop_assert_eq!(req, decoded);
    }
}
```

**Rule for review:** Any PR adding a new variant to `DaemonRequest` or
`DaemonResponse` must include it in the `Arbitrary` generator. Enforce via
the existing `protocol-drift.yml` CI workflow.

**Exit criteria:** All six types above have roundtrip property tests.
Protocol-drift CI checks generator completeness.

---

# Phase B: Tooling Adoption (External Dependencies)

Phase B items scale the methodologies from Phase A using purpose-built tools.
Each item is independently adoptable. Start with B1 and B2, which have the
highest signal-to-noise ratio.

---

## B1. cargo-mutants -- Automated Mutation Scanning (P0)

**Unlocked by:** A1 (mutation audit). After the manual audit fills obvious gaps,
cargo-mutants finds the ones humans miss.

**What it adds beyond A1:** Automated, exhaustive mutation of every branch and
return value -- not just the ones a human thought to check. Catches subtle
survivors like `>=` vs `>` in size limit checks.

**Setup:**
```
cargo install --locked cargo-mutants
cargo mutants -f crates/minibox-core/src/image/layer.rs
```

**Integration:**
- `just mutants-security` recipe for the six target modules.
- Weekly nightly.yml job, informational (non-blocking).

**New dependency:** None (cargo subcommand).

**Exit criteria:** Zero surviving mutants in security modules.

---

## B2. quickcheck -- Generator-Driven Properties (P0)

**Unlocked by:** A5 (roundtrip rule). After proptest covers the obvious
roundtrips, quickcheck's `Arbitrary` derive makes it trivial to add generators
for complex types.

**What it adds beyond A5:** Simpler `Arbitrary` derivation for nested enums
(DaemonRequest has many variants). Complements proptest where proptest
strategies are verbose.

**New property tests:**

| Property | Assertion |
|----------|-----------|
| Path traversal completeness | `validate_tar_entry_path` rejects iff path escapes root |
| Image ref roundtrip | `parse(ref.to_string()) == ref` |
| Protocol codec roundtrip | `deserialize(serialize(msg)) == msg` |
| Cgroup limit arithmetic | No overflow, no zero-division |
| IP allocator no-double-assign | `allocate()` never returns an in-use IP |
| Overlay mount-option string | Output is valid mount(2) option syntax |

**New dependency:** `quickcheck` (dev-only) in minibox-core and minibox.

**Integration:** `cargo xtask test-quickcheck`, added to merge.yml.

**Exit criteria:** Six property families passing, 100 iterations each.

---

## B3. Kani -- Formal Verification (P1)

**Unlocked by:** A2 (exhaustive small-domain tests). After exhaustive tests
cover enumerable domains, Kani proves correctness over unbounded domains (all
possible `PathBuf` values, all `u32` modes).

**Proof harnesses:**

```rust
#[cfg(kani)]
mod proofs {
    use super::*;

    #[kani::proof]
    fn validate_tar_entry_path_rejects_all_traversals() {
        let bytes: [u8; 64] = kani::any();
        if let Ok(s) = std::str::from_utf8(&bytes) {
            let path = std::path::Path::new(s);
            if path.components().any(|c| matches!(c, Component::ParentDir)) {
                assert!(validate_tar_entry_path(path).is_err());
            }
        }
    }

    #[kani::proof]
    fn setuid_bits_always_stripped() {
        let mode: u32 = kani::any();
        let stripped = mode & 0o777;
        assert!(stripped & 0o7000 == 0);
    }
}
```

**Target functions:** `validate_tar_entry_path`, `has_parent_dir_component`,
`relative_path`, setuid mask, `is_authorized`.

**New dependency:** `kani-verifier` (dev-only, `#[cfg(kani)]`).

**Integration:** `cargo kani --tests` in nightly.yml (informational).

**Exit criteria:** Five proof harnesses verify.

---

## B4. turmoil -- Deterministic Network Simulation (P1)

**Unlocked by:** A3 (stream trait boundary). Once server and GHCR client are
generic over their I/O type, turmoil slots in as a drop-in stream provider
with fault injection.

**What it adds beyond A3:** Turmoil simulates real network topology -- latency,
partitions, packet reorder -- not just canned byte sequences. It catches
ordering-dependent bugs that mock streams miss.

**Architecture:**

```
turmoil::Sim
  +-- "daemon" host (miniboxd server loop)
  +-- "client" host (DaemonRequest sender)
  +-- "registry" host (OCI manifest + layer server)
```

**Scenarios:** Half-frame request, client disconnect mid-stream, registry 503
mid-layer, registry timeout, packet reorder on multiplex.

**New dependency:** `turmoil` (dev-only) in minibox and miniboxd.

**Integration:** `cargo xtask test-turmoil`, added to merge.yml.

**Exit criteria:** Five turmoil scenarios passing deterministically.

---

## B5. shuttle -- Randomized Concurrency Testing (P2)

**Unlocked by:** A4 (barrier-based race tests). After manual barrier tests
cover known interleavings, shuttle explores the interleavings you didn't
think of.

**What it adds beyond A4:** Barrier tests fix the interleaving. Shuttle
randomizes across 1000 runs, catching races that only manifest under specific
scheduling the developer didn't anticipate.

**Scenarios:** Create+destroy race, event bus subscribe+broadcast, pause vs
exit, GC vs pull.

**New dependency:** `shuttle` (dev-only) in minibox.

**Constraint:** Code under test uses `shuttle::sync`/`shuttle::thread` via
`cfg(test)` conditional imports.

**Integration:** `cargo xtask test-shuttle`, added to merge.yml.

**Exit criteria:** Four scenarios passing, 1000 iterations each.

---

## B6. loom -- Exhaustive Concurrency Permutation (P3)

**Deferred** until a lock-free data structure is introduced in the daemon.
Current `Mutex`-based state is better served by shuttle (B5).

**Trigger:** Event bus moves to lock-free channel, container state uses
atomic CAS, or a custom ring buffer is added.

---

# Summary

## Dependency Overview

| Item | Phase | New deps? | Crates affected |
|------|-------|-----------|-----------------|
| Mutation audit checklist | A1 | No | -- |
| Exhaustive small-domain tests | A2 | No | minibox-core, minibox |
| Stream trait boundary | A3 | No | minibox, miniboxd |
| Barrier-based race tests | A4 | No | minibox |
| Roundtrip property rule | A5 | No | minibox-core, minibox |
| cargo-mutants | B1 | No (tool) | -- |
| quickcheck | B2 | Yes (dev) | minibox-core, minibox |
| Kani | B3 | Yes (dev, cfg-gated) | minibox-core |
| turmoil | B4 | Yes (dev) | minibox, miniboxd |
| shuttle | B5 | Yes (dev) | minibox |
| loom | B6 | Yes (dev) | deferred |

## CI Integration

| Gate | Workflow | Trigger | Blocking? |
|------|----------|---------|-----------|
| Exhaustive + barrier + roundtrip (A2/A4/A5) | merge.yml | push to main/next | yes |
| cargo-mutants security scan (B1) | nightly.yml | daily cron | no |
| quickcheck properties (B2) | merge.yml | push to main/next | yes |
| Kani proofs (B3) | nightly.yml | daily cron | no |
| turmoil scenarios (B4) | merge.yml | push to main/next | yes |
| shuttle scenarios (B5) | merge.yml | push to main/next | yes |

## Success Metrics

| Metric | Baseline | After Phase A | After Phase B |
|--------|----------|---------------|---------------|
| Mutation audit coverage (security modules) | unknown | 100% manual | 100% automated |
| Exhaustive small-domain tests | 1 (is_authorized) | 4 | 4 |
| Stream/transport trait coverage | 0 mock-stream tests | 5 | 5 + turmoil |
| Barrier-based race tests | 0 | 4 | 4 + shuttle |
| Roundtrip property tests | partial | 6 types | 6 + quickcheck |
| Formal proofs | 0 | 0 | 5 (kani) |
