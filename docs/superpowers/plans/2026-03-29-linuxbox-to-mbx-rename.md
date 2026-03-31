# mbx → mbx Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the `mbx` crate to `mbx` — directory, crate name, and all references workspace-wide.

**Architecture:** Pure mechanical rename. No behavioral changes. `crates/mbx/` becomes `crates/mbx/`, crate name changes from `mbx` to `mbx`, all `use mbx::` and `mbx = { ... }` references updated.

**Tech Stack:** Rust workspace, cargo, git mv

---

## File Map

**Renamed:**

- `crates/mbx/` → `crates/mbx/` (entire directory, via `git mv`)

**Modified:**

- `crates/mbx/Cargo.toml` — change `name`
- `Cargo.toml` (workspace root) — member path + workspace dep
- `crates/miniboxd/Cargo.toml`
- `crates/macbox/Cargo.toml`
- `crates/daemonbox/Cargo.toml`
- `crates/minibox-bench/Cargo.toml`
- `crates/winbox/Cargo.toml`
- `crates/xtask/src/main.rs` — `-p mbx` flags in command strings
- `CLAUDE.md` — all prose and command references
- All `.rs` files with `use mbx::` or `mbx::` qualified paths (~45 files)

---

### Task 1: Move the directory

**Files:**

- Move: `crates/mbx/` → `crates/mbx/`

- [ ] **Step 1: git mv the crate directory**

```bash
git mv crates/mbx crates/mbx
```

- [ ] **Step 2: Verify the move**

```bash
ls crates/mbx/
```

Expected: `Cargo.toml  README.md  benches/  examples/  src/  tests/`

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "chore: git mv crates/mbx → crates/mbx"
```

---

### Task 2: Update crate manifests

**Files:**

- Modify: `crates/mbx/Cargo.toml`
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/miniboxd/Cargo.toml`
- Modify: `crates/macbox/Cargo.toml`
- Modify: `crates/daemonbox/Cargo.toml`
- Modify: `crates/minibox-bench/Cargo.toml`
- Modify: `crates/winbox/Cargo.toml`

- [ ] **Step 1: Rename the crate in its own manifest**

In `crates/mbx/Cargo.toml`, change:

```toml
name = "mbx"
```

to:

```toml
name = "mbx"
```

- [ ] **Step 2: Update workspace root manifest**

In `Cargo.toml`, change:

```toml
"crates/mbx",
```

to:

```toml
"crates/mbx",
```

And change:

```toml
mbx = { path = "crates/mbx" }
```

to:

```toml
mbx = { path = "crates/mbx" }
```

- [ ] **Step 3: Update miniboxd**

In `crates/miniboxd/Cargo.toml`, change:

```toml
mbx = { workspace = true }
```

to:

```toml
mbx = { workspace = true }
```

- [ ] **Step 4: Update macbox**

In `crates/macbox/Cargo.toml`, change:

```toml
mbx = { workspace = true }
```

to:

```toml
mbx = { workspace = true }
```

- [ ] **Step 5: Update daemonbox**

In `crates/daemonbox/Cargo.toml`, change:

```toml
mbx = { workspace = true }
```

to:

```toml
mbx = { workspace = true }
```

- [ ] **Step 6: Update minibox-bench**

In `crates/minibox-bench/Cargo.toml`, change:

```toml
mbx.workspace = true
```

to:

```toml
mbx.workspace = true
```

- [ ] **Step 7: Update winbox (dep + ignored list)**

In `crates/winbox/Cargo.toml`, change:

```toml
mbx = { workspace = true }
```

to:

```toml
mbx = { workspace = true }
```

And in the `ignored` list:

```toml
ignored = ["daemonbox", "mbx", "tokio", "tracing-subscriber"]
```

to:

```toml
ignored = ["daemonbox", "mbx", "tokio", "tracing-subscriber"]
```

- [ ] **Step 8: Verify cargo resolves**

```bash
cargo check -p mbx 2>&1 | head -20
```

Expected: no "package not found" errors (compile errors about `use mbx::` in .rs files are expected at this stage)

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml crates/mbx/Cargo.toml crates/miniboxd/Cargo.toml crates/macbox/Cargo.toml crates/daemonbox/Cargo.toml crates/minibox-bench/Cargo.toml crates/winbox/Cargo.toml
git commit -m "chore: rename mbx → mbx in all Cargo.toml manifests"
```

---

### Task 3: Update Rust source files

**Files:**

- Modify: all `.rs` files containing `mbx` (~45 files)

This is a workspace-wide search-and-replace of the identifier `mbx` with `mbx` in `.rs` files.

- [ ] **Step 1: Replace all `use mbx::` with `use mbx::`**

```bash
find crates -name "*.rs" | xargs sed -i '' 's/use mbx::/use mbx::/g'
```

- [ ] **Step 2: Replace all `mbx::` qualified paths**

```bash
find crates -name "*.rs" | xargs sed -i '' 's/mbx::/mbx::/g'
```

- [ ] **Step 3: Replace remaining bare `mbx` identifiers (extern crate, feature flags, doc comments)**

```bash
find crates -name "*.rs" | xargs sed -i '' 's/\bmbx\b/mbx/g'
```

- [ ] **Step 4: Verify no `mbx` remains in .rs files**

```bash
grep -r "mbx" crates --include="*.rs"
```

Expected: no output

- [ ] **Step 5: cargo check to confirm it compiles**

```bash
cargo check --workspace 2>&1 | tail -5
```

Expected: `Finished` or only unrelated warnings

- [ ] **Step 6: Commit**

```bash
git add -A
git commit -m "chore: update all .rs files mbx → mbx"
```

---

### Task 4: Update xtask command strings

**Files:**

- Modify: `crates/xtask/src/main.rs`

- [ ] **Step 1: Replace `-p mbx` with `-p mbx` in all command strings**

In `crates/xtask/src/main.rs`, change every occurrence of `-p mbx` to `-p mbx`. There are 7 occurrences — lines 83, 88, 98, 104, 120, 138, 140. The result should be:

Line 83:

```rust
"cargo clippy -p mbx -p minibox-macros -p minibox-cli -p daemonbox -p macbox -p miniboxd -- -D warnings"
```

Line 88:

```rust
"cargo build --release -p mbx -p minibox-macros -p minibox-cli -p daemonbox -p minibox-bench"
```

Line 98:

```rust
"cargo nextest run --release -p mbx -p minibox-macros -p minibox-cli -p daemonbox"
```

Line 104:

```rust
"cargo llvm-cov nextest -p mbx -p minibox-macros -p minibox-cli -p daemonbox --html"
```

Line 120:

```rust
"cargo test --release -p mbx -p minibox-macros -p minibox-cli -p daemonbox --lib"
```

Line 138:

```rust
cmd!(sh, "cargo test --release -p mbx --test proptest_suite")
```

Line 140:

```rust
.context("mbx property tests failed")?;
```

- [ ] **Step 2: Verify**

```bash
grep "mbx" crates/xtask/src/main.rs
```

Expected: no output

- [ ] **Step 3: Commit**

```bash
git add crates/xtask/src/main.rs
git commit -m "chore: update xtask command strings mbx → mbx"
```

---

### Task 5: Update CLAUDE.md

**Files:**

- Modify: `CLAUDE.md`

- [ ] **Step 1: Replace all `mbx` references in CLAUDE.md**

```bash
sed -i '' 's/mbx/mbx/g' CLAUDE.md
```

- [ ] **Step 2: Fix any prose that now reads awkwardly**

Check that the naming convention note still makes sense. It currently reads:

> Platform crates follow the `{platform}box` naming convention: `mbx` (Linux namespaces/cgroups), `macbox` (macOS Colima), `winbox` (Windows stub).

After the rename this should read:

> Platform crates follow the `{platform}box` naming convention: `mbx` (Linux namespaces/cgroups), `macbox` (macOS Colima), `winbox` (Windows stub).

That's fine — update accordingly if needed.

- [ ] **Step 3: Verify**

```bash
grep "mbx" CLAUDE.md
```

Expected: no output

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md mbx → mbx"
```

---

### Task 6: Final verification gate

- [ ] **Step 1: Full workspace check**

```bash
cargo check --workspace
```

Expected: `Finished` with no errors

- [ ] **Step 2: Clippy**

```bash
cargo clippy --workspace -- -D warnings
```

Expected: no errors (warnings-as-errors)

- [ ] **Step 3: Unit tests**

```bash
cargo xtask test-unit
```

Expected: all pass

- [ ] **Step 4: Confirm no remaining `mbx` references**

```bash
grep -r "mbx" . --include="*.rs" --include="*.toml" --include="*.md" --exclude-dir=".git" --exclude-dir="target"
```

Expected: only matches inside `docs/superpowers/` (spec/plan docs) and git history — no source references

- [ ] **Step 5: Final commit if anything was auto-formatted**

```bash
git status
```

If `cargo fmt` auto-ran on any files during the above steps, commit them:

```bash
git add -A
git commit -m "chore: cargo fmt after mbx rename"
```
