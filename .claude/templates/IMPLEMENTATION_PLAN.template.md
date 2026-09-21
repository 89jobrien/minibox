# <Ticket ID> <Short Name> — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use godmode:task-driven-development  
> (recommended) or godmode:task-management to implement this plan.  
> Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** <1–2 lines describing the concrete outcome, including key behaviors, tests, and guardrails to be added or fixed.>

**Architecture:** <Brief note on the relevant architecture style (e.g., SOLID hexagonal), ports/adapters involved, and what boundary the plan is locking down.>

**Tech Stack:** <Languages, tools, frameworks, CLI commands — e.g., Rust, `cargo nextest`, `syn`, `git rebase`.>

---

## Causal Chain

```text
T1: <Task 1 name>                         (prereq for base state)
  └─► T2: <Task 2 name>                   (unblocks all below)
        ├─► T3: <Task 3 name>             (correctness prerequisite)
        │     └─► T4: <Task 4 name>
        └─► T5: <Task 5 name>
              └─► T6: <Task 6 name>
                    └─► T7: <Task 7 name>
                          └─► T8: <Task 8 name>
```

**Note:** <Explain key dependencies and which tasks can proceed in parallel once prerequisites are met.>

---

## File Map

| Action    | Path                               |
| --------- | ---------------------------------- |
| Modify    | `<crate-or-app>/src/<file>.rs`     |
| Create    | `<crate-or-app>/tests/<file>.rs`   |
| Modify    | `<crate-or-app>/tests/main.rs`     |
| Reference | `<crate-or-app>/src/<boundary>.rs` |
| Reference | `<other crate>/tests/<pattern>.rs` |

---

## Task 1: <Task 1 title>

**Files:** <“none (git operations only)” or list of files>

- [ ] **Step 1: <short description>**

  ```bash
  # commands
  ```

- [ ] **Step 2: <short description>**

  ```bash
  # commands
  ```

- [ ] **Step 3: <short description>**

  ```bash
  # commands
  ```

---

## Task 2: <Task 2 title>

**Files:** <list or “none”>

- [ ] **Step 1: <description>**

  ```bash
  # commands
  ```

- [ ] **Step 2: <description>**

  ```bash
  # commands
  ```

- [ ] **Step 3: <description>**

  ```bash
  # commands
  ```

---

## Task 3: <Bug fix or feature task title>

**Files:**

- Modify: `<path/to/file.rs>`
- Create: `<optional new file>`

**Bug/Change:** <Plain-language description of what’s wrong or what behavior you’re adding.>

**Fix/Implementation:** <Brief explanation of the intended change in terms of behavior or invariants.>

- [ ] **Step 1: Write the failing test (red)**

  <Explain where to put the test, and add a code block with the new test(s).>

  ```rust
  #[test]
  fn <test_name_describes_invariant>() {
      // arrange / act / assert
  }
  ```

- [ ] **Step 2: Run to confirm red**

  ```bash
  cargo nextest run -p <crate> -E 'test(<pattern>)'
  # expected: FAIL – current implementation violates the invariant
  ```

- [ ] **Step 3: Implement the change**

  <Describe which function/loop/branch you’re changing and show the new code.>

  ```rust
  // updated implementation
  ```

- [ ] **Step 4: Run focused tests (green)**

  ```bash
  cargo nextest run -p <crate> -E 'test(<pattern>)'
  # expected: all pass
  ```

---

## Task 4: Commit <short description>

- [ ] **Step 1: Stage and commit**

  ```bash
  git add <paths>
  git commit -m "<type(scope)): <summary>

  <Extended body explaining what was changed and why, including ticket reference.>"
  ```

---

## Task 5: Add <contract tests / feature> for <boundary>

**Files:**

- Modify: `<crate>/Cargo.toml`
- Modify: `<crate>/src/<boundary>.rs`
- Create: `<crate>/tests/conformance/<boundary>_contract.rs`
- Modify: `<crate>/tests/conformance/main.rs`

<Explain why integration tests need a feature flag or visibility adjustment, and which port type/func they target.>

- [ ] **Step 1: Write failing contract tests (red)**

  ```rust
  //! Conformance tests: <boundary> port contract.
  //!
  //! Verifies that <port> satisfies:
  //! - <invariant>
  //! - <invariant>
  //! - <invariant>

  use <crate>::<module>::{<symbols>};

  #[test]
  fn <test_name>() {
      // arrange
      // act
      // assert
  }
  ```

- [ ] **Step 2: Wire into conformance test harness**

  ```rust
  // in <crate>/tests/conformance/main.rs
  mod <boundary>_contract;
  ```

- [ ] **Step 3: Run to confirm red (visibility / linkage)**

  ```bash
  cargo nextest run -p <crate> --test conformance -E 'test(<pattern>)'
  # expected: compile error or failing tests, confirming boundary not yet wired
  ```

- [ ] **Step 4: Add feature/visibility for test-utils**

  ```toml
  # Cargo.toml
  [features]
  test-utils = []
  ```

  ```rust
  // In boundary file
  #[cfg(any(test, feature = "test-utils"))]
  pub fn <test_double_fn>(...) { ... }
  ```

- [ ] **Step 5: Run contract tests (green)**

  ```bash
  cargo nextest run -p <crate> --test conformance --features test-utils
  # expected: new contract tests pass
  ```

---

## Task 6: Commit <contract tests>

- [ ] **Step 1: Stage and commit**

  ```bash
  git add <paths>
  git commit -m "test(conformance): add <boundary> contract tests

  <Bulleted summary of what invariants are now locked in.>"
  ```

---

## Task 7: Add <AST/source> guardrail for <boundary>

**Files:**

- Create: `<crate>/tests/conformance/<boundary>_guardrails.rs`
- Modify: `<crate>/tests/conformance/main.rs`

<Explain which source file you parse with `syn` and what structural invariants you assert (public fn, enum shape, variants, no duplicate traits, etc.).>

- [ ] **Step 1: Ensure `syn` dev-dep**

  ```toml
  [dev-dependencies]
  syn = { version = "2", features = ["full", "visit"] }
  ```

- [ ] **Step 2: Write failing guardrail test (red)**

  ```rust
  use std::fs;
  use std::path::Path;
  use syn::{File, Item, ItemFn, ItemEnum, Visibility};

  fn boundary_source() -> File {
      let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/<boundary>.rs");
      let src = fs::read_to_string(&path).expect("<boundary>.rs readable");
      syn::parse_file(&src).expect("<boundary>.rs parses")
  }

  #[test]
  fn <fn_name>_is_public_function() {
      let file = boundary_source();
      let found = file.items.iter().any(|item| matches!(item, Item::Fn(ItemFn { vis: Visibility::Public(_), sig, .. }) if sig.ident == "<fn_name>"));
      assert!(found, "expected public fn <fn_name> in <boundary>.rs");
  }
  ```

- [ ] **Step 3: Wire into conformance harness**

  ```rust
  // in <crate>/tests/conformance/main.rs
  mod <boundary>_guardrails;
  ```

- [ ] **Step 4: Run to confirm red, then fix and re-run to green**

  ```bash
  cargo nextest run -p <crate> --test conformance --features test-utils \
    -E 'test(<fn_name>_is_public_function)'
  ```

---

## Task 8: Commit guardrails and open PR

- [ ] **Step 1: Run full test suite**

  ```bash
  cargo nextest run --workspace
  cargo nextest run -p <crate> --test conformance --features test-utils
  ```

- [ ] **Step 2: Stage and commit**

  ```bash
  git add <paths>
  git commit -m "test(conformance): add AST guardrail for <boundary>

  <Short summary of what the guardrail asserts and why.>"
  ```

- [ ] **Step 3: Push and open PR**

  ```bash
  git push origin <branch>
  gh pr create \
    --title "<type(conformance): short PR title>" \
    --body "$(cat <<'EOF'
  ## Summary
  - <bullet 1>
  - <bullet 2>
  - <bullet 3>

  ## Test plan
  - [ ] cargo nextest run -p <crate> --test conformance --features test-utils
  - [ ] cargo nextest run --workspace
  - [ ] CI green
  EOF
  )"
  ```

---

## Self-Review

**Spec coverage check:**

| Gap / objective     | Task |
| ------------------- | ---- |
| <gap 1 description> | T?   |
| <gap 2 description> | T?   |
| <gap 3 description> | T?   |

**Placeholder scan:** Confirm all `<placeholders>` have been filled with concrete commands, paths, and identifiers.

**Type consistency:** Verify all referenced types/functions (`<TypeName>`, `<fn_name>`, feature flags, module paths) match the actual code in `<crate>/src`.
