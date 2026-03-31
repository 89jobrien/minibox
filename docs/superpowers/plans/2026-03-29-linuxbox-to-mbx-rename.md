# linuxbox → mbx Rename Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rename the `linuxbox` crate to `mbx` — directory, crate name, and all references workspace-wide.

**Architecture:** Pure mechanical rename. No behavioral changes. `crates/linuxbox/` becomes `crates/mbx/`, crate name changes from `linuxbox` to `mbx`, all `use linuxbox::` and `linuxbox = { ... }` references updated.

**Tech Stack:** Rust workspace, cargo, git mv

---

## File Map

**Renamed:**
- `crates/linuxbox/` → `crates/mbx/` (entire directory, via `git mv`)

**Modified:**
- `crates/mbx/Cargo.toml` — change `name`
- `Cargo.toml` (workspace root) — member path + workspace dep
- `crates/miniboxd/Cargo.toml`
- `crates/macbox/Cargo.toml`
- `crates/daemonbox/Cargo.toml`
- `crates/minibox-bench/Cargo.toml`
- `crates/winbox/Cargo.toml`
- `crates/xtask/src/main.rs` — `-p linuxbox` flags in command strings
- `CLAUDE.md` — all prose and command references
- All `.rs` files with `use linuxbox::` or `linuxbox::` qualified paths (~45 files)

---

### Task 1: Move the directory

**Files:**
- Move: `crates/linuxbox/` → `crates/mbx/`

- [ ] **Step 1: git mv the crate directory**

```bash
git mv crates/linuxbox crates/mbx
```

- [ ] **Step 2: Verify the move**

```bash
ls crates/mbx/
```
Expected: `Cargo.toml  README.md  benches/  examples/  src/  tests/`

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "chore: git mv crates/linuxbox → crates/mbx"
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
name = "linuxbox"
```
to:
```toml
name = "mbx"
```

- [ ] **Step 2: Update workspace root manifest**

In `Cargo.toml`, change:
```toml
"crates/linuxbox",
```
to:
```toml
"crates/mbx",
```

And change:
```toml
linuxbox = { path = "crates/linuxbox" }
```
to:
```toml
mbx = { path = "crates/mbx" }
```

- [ ] **Step 3: Update miniboxd**

In `crates/miniboxd/Cargo.toml`, change:
```toml
linuxbox = { workspace = true }
```
to:
```toml
mbx = { workspace = true }
```

- [ ] **Step 4: Update macbox**

In `crates/macbox/Cargo.toml`, change:
```toml
linuxbox = { workspace = true }
```
to:
```toml
mbx = { workspace = true }
```

- [ ] **Step 5: Update daemonbox**

In `crates/daemonbox/Cargo.toml`, change:
```toml
linuxbox = { workspace = true }
```
to:
```toml
mbx = { workspace = true }
```

- [ ] **Step 6: Update minibox-bench**

In `crates/minibox-bench/Cargo.toml`, change:
```toml
linuxbox.workspace = true
```
to:
```toml
mbx.workspace = true
```

- [ ] **Step 7: Update winbox (dep + ignored list)**

In `crates/winbox/Cargo.toml`, change:
```toml
linuxbox = { workspace = true }
```
to:
```toml
mbx = { workspace = true }
```

And in the `ignored` list:
```toml
ignored = ["daemonbox", "linuxbox", "tokio", "tracing-subscriber"]
```
to:
```toml
ignored = ["daemonbox", "mbx", "tokio", "tracing-subscriber"]
```

- [ ] **Step 8: Verify cargo resolves**

```bash
cargo check -p mbx 2>&1 | head -20
```
Expected: no "package not found" errors (compile errors about `use linuxbox::` in .rs files are expected at this stage)

- [ ] **Step 9: Commit**

```bash
git add Cargo.toml crates/mbx/Cargo.toml crates/miniboxd/Cargo.toml crates/macbox/Cargo.toml crates/daemonbox/Cargo.toml crates/minibox-bench/Cargo.toml crates/winbox/Cargo.toml
git commit -m "chore: rename linuxbox → mbx in all Cargo.toml manifests"
```

---

### Task 3: Update Rust source files

**Files:**
- Modify: all `.rs` files containing `linuxbox` (~45 files)

This is a workspace-wide search-and-replace of the identifier `linuxbox` with `mbx` in `.rs` files.

- [ ] **Step 1: Replace all `use linuxbox::` with `use mbx::`**

```bash
find crates -name "*.rs" | xargs sed -i '' 's/use linuxbox::/use mbx::/g'
```

- [ ] **Step 2: Replace all `linuxbox::` qualified paths**

```bash
find crates -name "*.rs" | xargs sed -i '' 's/linuxbox::/mbx::/g'
```

- [ ] **Step 3: Replace remaining bare `linuxbox` identifiers (extern crate, feature flags, doc comments)**

```bash
find crates -name "*.rs" | xargs sed -i '' 's/\blinuxbox\b/mbx/g'
```

- [ ] **Step 4: Verify no `linuxbox` remains in .rs files**

```bash
grep -r "linuxbox" crates --include="*.rs"
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
git commit -m "chore: update all .rs files linuxbox → mbx"
```

---

### Task 4: Update xtask command strings

**Files:**
- Modify: `crates/xtask/src/main.rs`

- [ ] **Step 1: Replace `-p linuxbox` with `-p mbx` in all command strings**

In `crates/xtask/src/main.rs`, change every occurrence of `-p linuxbox` to `-p mbx`. There are 7 occurrences — lines 83, 88, 98, 104, 120, 138, 140. The result should be:

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
grep "linuxbox" crates/xtask/src/main.rs
```
Expected: no output

- [ ] **Step 3: Commit**

```bash
git add crates/xtask/src/main.rs
git commit -m "chore: update xtask command strings linuxbox → mbx"
```

---

### Task 5: Update CLAUDE.md

**Files:**
- Modify: `CLAUDE.md`

- [ ] **Step 1: Replace all `linuxbox` references in CLAUDE.md**

```bash
sed -i '' 's/linuxbox/mbx/g' CLAUDE.md
```

- [ ] **Step 2: Fix any prose that now reads awkwardly**

Check that the naming convention note still makes sense. It currently reads:
> Platform crates follow the `{platform}box` naming convention: `linuxbox` (Linux namespaces/cgroups), `macbox` (macOS Colima), `winbox` (Windows stub).

After the rename this should read:
> Platform crates follow the `{platform}box` naming convention: `mbx` (Linux namespaces/cgroups), `macbox` (macOS Colima), `winbox` (Windows stub).

That's fine — update accordingly if needed.

- [ ] **Step 3: Verify**

```bash
grep "linuxbox" CLAUDE.md
```
Expected: no output

- [ ] **Step 4: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update CLAUDE.md linuxbox → mbx"
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

- [ ] **Step 4: Confirm no remaining `linuxbox` references**

```bash
grep -r "linuxbox" . --include="*.rs" --include="*.toml" --include="*.md" --exclude-dir=".git" --exclude-dir="target"
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
