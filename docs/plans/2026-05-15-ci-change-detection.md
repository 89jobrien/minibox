# Plan: CI Change Detection

## Goal

Add `cargo xtask detect-changes <base-ref>` so CI workflows can skip irrelevant jobs and
route to the correct test suite based on which workspace areas actually changed.

## Architecture

- **Crates affected**: `xtask` only
- **New types**:
  - `Area` enum — 10 variants, one per workspace area
  - `ChangeSet` struct — 10 `bool` fields, one per `Area`
- **Data flow**:
  `git diff --name-only <base>...HEAD` → lines → `classify_path()` → `ChangeSet`
  → `emit_gha_outputs()` → `$GITHUB_OUTPUT` (or stdout)
- **Workflows updated**: `pr.yml`, `conformance.yml`, `merge.yml`

## Tech Stack

- Rust 2024 edition, `xshell` (already in xtask) for `git diff` subprocess
- No new dependencies

## Tasks

---

### Task 1: Delete the copied detect_changes.rs and write the Area classifier

**Crate**: `xtask`
**File(s)**: `xtask/src/detect_changes.rs`
**Run**: `cargo nextest run -p xtask`

1. Write failing tests first — replace the file entirely with:

```rust
//! CI change detection: classify changed paths into workspace areas.

use anyhow::Result;
use std::path::Path;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Area {
    Core,
    Daemon,
    Cli,
    Runtime,
    Macbox,
    Winbox,
    Conformance,
    Xtask,
    Docs,
    Workflows,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct ChangeSet {
    pub core:        bool,
    pub daemon:      bool,
    pub cli:         bool,
    pub runtime:     bool,
    pub macbox:      bool,
    pub winbox:      bool,
    pub conformance: bool,
    pub xtask:       bool,
    pub docs:        bool,
    pub workflows:   bool,
}

impl ChangeSet {
    fn set(&mut self, area: Area) {
        match area {
            Area::Core        => self.core        = true,
            Area::Daemon      => self.daemon      = true,
            Area::Cli         => self.cli         = true,
            Area::Runtime     => self.runtime     = true,
            Area::Macbox      => self.macbox      = true,
            Area::Winbox      => self.winbox      = true,
            Area::Conformance => self.conformance = true,
            Area::Xtask       => self.xtask       = true,
            Area::Docs        => self.docs        = true,
            Area::Workflows   => self.workflows   = true,
        }
    }
}

// ---------------------------------------------------------------------------
// Path classifier
// ---------------------------------------------------------------------------

/// Map a changed file path (relative to workspace root) to a workspace area.
///
/// Returns `None` for paths that don't match any tracked area (e.g. `fuzz/`).
pub fn classify_path(path: &str) -> Option<Area> {
    if path.starts_with("crates/minibox-core/")
        || path.starts_with("crates/minibox-macros/")
    {
        Some(Area::Core)
    } else if path.starts_with("crates/miniboxd/") {
        Some(Area::Daemon)
    } else if path.starts_with("crates/mbx/") {
        Some(Area::Cli)
    } else if path.starts_with("crates/minibox/") {
        Some(Area::Runtime)
    } else if path.starts_with("crates/macbox/") {
        Some(Area::Macbox)
    } else if path.starts_with("crates/winbox/") {
        Some(Area::Winbox)
    } else if path.starts_with("crates/minibox-conformance/")
        || path.starts_with("crates/minibox-crux-plugin/")
    {
        Some(Area::Conformance)
    } else if path.starts_with("xtask/") {
        Some(Area::Xtask)
    } else if path.starts_with("docs/")
        || (path.ends_with(".md") && !path.contains('/'))
    {
        Some(Area::Docs)
    } else if path.starts_with(".github/") {
        Some(Area::Workflows)
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Run `git diff --name-only <base_ref>...HEAD` and classify changed paths.
pub fn detect_changes(root: &Path, base_ref: &str) -> Result<ChangeSet> {
    todo!("implement in task 2")
}

/// Write `key=value` lines to `$GITHUB_OUTPUT` if set, otherwise to stdout.
pub fn emit_gha_outputs(cs: &ChangeSet) -> Result<()> {
    todo!("implement in task 3")
}

pub fn run(root: &Path, base_ref: &str) -> Result<()> {
    let cs = detect_changes(root, base_ref)?;
    emit_gha_outputs(&cs)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_minibox_core() {
        assert_eq!(
            classify_path("crates/minibox-core/src/domain.rs"),
            Some(Area::Core)
        );
    }

    #[test]
    fn classify_minibox_macros() {
        assert_eq!(
            classify_path("crates/minibox-macros/src/lib.rs"),
            Some(Area::Core)
        );
    }

    #[test]
    fn classify_miniboxd() {
        assert_eq!(
            classify_path("crates/miniboxd/src/handler.rs"),
            Some(Area::Daemon)
        );
    }

    #[test]
    fn classify_mbx_cli() {
        assert_eq!(
            classify_path("crates/mbx/src/main.rs"),
            Some(Area::Cli)
        );
    }

    #[test]
    fn classify_minibox_runtime() {
        assert_eq!(
            classify_path("crates/minibox/src/adapters/docker.rs"),
            Some(Area::Runtime)
        );
    }

    #[test]
    fn classify_macbox() {
        assert_eq!(
            classify_path("crates/macbox/src/krun.rs"),
            Some(Area::Macbox)
        );
    }

    #[test]
    fn classify_winbox() {
        assert_eq!(
            classify_path("crates/winbox/src/lib.rs"),
            Some(Area::Winbox)
        );
    }

    #[test]
    fn classify_conformance() {
        assert_eq!(
            classify_path("crates/minibox-conformance/src/lib.rs"),
            Some(Area::Conformance)
        );
    }

    #[test]
    fn classify_xtask() {
        assert_eq!(
            classify_path("xtask/src/gates.rs"),
            Some(Area::Xtask)
        );
    }

    #[test]
    fn classify_docs_subdir() {
        assert_eq!(
            classify_path("docs/ARCHITECTURE.mbx.md"),
            Some(Area::Docs)
        );
    }

    #[test]
    fn classify_root_md() {
        assert_eq!(classify_path("README.md"), Some(Area::Docs));
        assert_eq!(classify_path("CHANGELOG.md"), Some(Area::Docs));
    }

    #[test]
    fn classify_workflows() {
        assert_eq!(
            classify_path(".github/workflows/pr.yml"),
            Some(Area::Workflows)
        );
    }

    #[test]
    fn classify_unknown_returns_none() {
        assert_eq!(classify_path("fuzz/corpus/something"), None);
        assert_eq!(classify_path("Cargo.lock"), None);
        assert_eq!(classify_path("scripts/preflight.nu"), None);
    }

    #[test]
    fn changeset_folds_multiple_paths() {
        let paths = [
            "crates/minibox-core/src/protocol.rs",
            "crates/miniboxd/src/handler.rs",
            "docs/FEATURE_MATRIX.mbx.md",
        ];
        let mut cs = ChangeSet::default();
        for p in &paths {
            if let Some(area) = classify_path(p) {
                cs.set(area);
            }
        }
        assert!(cs.core);
        assert!(cs.daemon);
        assert!(cs.docs);
        assert!(!cs.cli);
        assert!(!cs.runtime);
    }
}
```

2. Run: `cargo nextest run -p xtask -- detect_changes`
   Expected: PASS (all `classify_path` tests pass; `todo!()` stubs not yet reached)

3. Run: `cargo clippy -p xtask -- -D warnings`
   Expected: zero warnings

4. Run: `git branch --show-current`
   Verify output is NOT `main`. Stop immediately if it is.
   Commit: `git commit -m "feat(xtask): add Area classifier and ChangeSet for CI change detection"`

---

### Task 2: Implement detect_changes() with git diff

**Crate**: `xtask`
**File(s)**: `xtask/src/detect_changes.rs`
**Run**: `cargo nextest run -p xtask -- detect_changes`

1. Add an integration test at the bottom of `detect_changes.rs` (inside `#[cfg(test)]`):

```rust
    #[test]
    fn detect_changes_with_real_git() {
        use std::process::Command;
        use tempfile::TempDir;

        // Set up a temp git repo with two commits.
        let dir = TempDir::new().expect("tempdir");
        let root = dir.path();

        let git = |args: &[&str]| {
            Command::new("git")
                .args(args)
                .current_dir(root)
                .output()
                .expect("git")
        };

        git(&["init", "-b", "main"]);
        git(&["config", "user.email", "test@test.com"]);
        git(&["config", "user.name", "Test"]);

        // First commit: a core file.
        std::fs::create_dir_all(root.join("crates/minibox-core/src"))
            .expect("mkdir");
        std::fs::write(root.join("crates/minibox-core/src/lib.rs"), b"// v1")
            .expect("write");
        git(&["add", "."]);
        git(&["commit", "-m", "initial"]);

        // Second commit: touch core + docs.
        std::fs::write(root.join("crates/minibox-core/src/lib.rs"), b"// v2")
            .expect("write");
        std::fs::create_dir_all(root.join("docs")).expect("mkdir");
        std::fs::write(root.join("docs/ARCHITECTURE.md"), b"# arch")
            .expect("write");
        git(&["add", "."]);
        git(&["commit", "-m", "update"]);

        let cs = detect_changes(root, "HEAD^").expect("detect_changes");
        assert!(cs.core,  "core should be true");
        assert!(cs.docs,  "docs should be true");
        assert!(!cs.daemon, "daemon should be false");
        assert!(!cs.cli,    "cli should be false");
    }
```

2. Add this import at the top of `detect_changes.rs`:

```rust
use xshell::{Shell, cmd};
```

3. Replace the `todo!()` in `detect_changes()` with:

```rust
pub fn detect_changes(root: &Path, base_ref: &str) -> Result<ChangeSet> {
    let sh = Shell::new()?;
    sh.change_dir(root);

    // Use three-dot range: shows all commits reachable from HEAD but not base_ref.
    // Falls back to two-dot if three-dot produces no output (e.g. first commit).
    let range = format!("{base_ref}...HEAD");
    let output = cmd!(sh, "git diff --name-only {range}").read()?;

    let mut cs = ChangeSet::default();
    for line in output.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if let Some(area) = classify_path(line) {
            cs.set(area);
        }
    }
    Ok(cs)
}
```

4. Add `tempfile` to `[dev-dependencies]` in `xtask/Cargo.toml` — it is already present,
   so no change needed.

5. Run: `cargo nextest run -p xtask -- detect_changes`
   Expected: all tests PASS including the integration test

6. Run: `cargo clippy -p xtask -- -D warnings`
   Expected: zero warnings

7. Run: `git branch --show-current`
   Verify output is NOT `main`. Stop immediately if it is.
   Commit: `git commit -m "feat(xtask): implement detect_changes() via git diff"`

---

### Task 3: Implement emit_gha_outputs() and wire xtask command

**Crate**: `xtask`
**File(s)**: `xtask/src/detect_changes.rs`, `xtask/src/main.rs`
**Run**: `cargo nextest run -p xtask`

1. Write a test for `emit_gha_outputs()` in `detect_changes.rs`:

```rust
    #[test]
    fn emit_outputs_formats_correctly() {
        // Capture stdout by calling the inner serialise function directly.
        let cs = ChangeSet {
            core: true,
            daemon: false,
            cli: true,
            runtime: false,
            macbox: false,
            winbox: false,
            conformance: false,
            xtask: false,
            docs: false,
            workflows: false,
        };
        let lines = changeset_to_output_lines(&cs);
        assert!(lines.contains(&"core=true".to_string()));
        assert!(lines.contains(&"daemon=false".to_string()));
        assert!(lines.contains(&"cli=true".to_string()));
    }
```

2. Add this helper and implement `emit_gha_outputs()`:

```rust
/// Serialise a ChangeSet to `key=value` output lines.
pub fn changeset_to_output_lines(cs: &ChangeSet) -> Vec<String> {
    vec![
        format!("core={}",        cs.core),
        format!("daemon={}",      cs.daemon),
        format!("cli={}",         cs.cli),
        format!("runtime={}",     cs.runtime),
        format!("macbox={}",      cs.macbox),
        format!("winbox={}",      cs.winbox),
        format!("conformance={}",  cs.conformance),
        format!("xtask={}",       cs.xtask),
        format!("docs={}",        cs.docs),
        format!("workflows={}",   cs.workflows),
    ]
}

/// Write outputs to `$GITHUB_OUTPUT` if set, otherwise print to stdout.
pub fn emit_gha_outputs(cs: &ChangeSet) -> Result<()> {
    use std::io::Write;

    let lines = changeset_to_output_lines(cs);

    if let Ok(output_path) = std::env::var("GITHUB_OUTPUT") {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&output_path)
            .with_context(|| format!("failed to open GITHUB_OUTPUT: {output_path}"))?;
        for line in &lines {
            writeln!(f, "{line}")?;
        }
    } else {
        for line in &lines {
            println!("{line}");
        }
    }
    Ok(())
}
```

3. Wire the command in `xtask/src/main.rs`. Add `mod detect_changes;` with the other
   module declarations, then add this arm to the `match`:

```rust
        Some("detect-changes") => {
            let base_ref = env::args().nth(2).unwrap_or_else(|| "HEAD^".to_string());
            detect_changes::run(root, &base_ref)
        }
```

   Also add it to the help text in the `None` arm:

```rust
            eprintln!(
                "  detect-changes [<base-ref>]  classify changed paths; emit GHA outputs (default base: HEAD^)"
            );
```

4. Run: `cargo nextest run -p xtask`
   Expected: all tests PASS

5. Run: `cargo clippy -p xtask -- -D warnings`
   Expected: zero warnings

6. Run: `cargo xtask detect-changes HEAD^` from the workspace root.
   Expected: prints `core=true/false` etc. lines to stdout (no crash).

7. Run: `git branch --show-current`
   Verify output is NOT `main`. Stop immediately if it is.
   Commit: `git commit -m "feat(xtask): emit GHA outputs and wire detect-changes command"`

---

### Task 4: Update pr.yml

**File(s)**: `.github/workflows/pr.yml`
**Run**: (manual GHA verification after push)

Replace `pr.yml` with:

```yaml
name: PR

on:
  pull_request:
    branches: [main, next]

env:
  CARGO_INCREMENTAL: 0

jobs:
  detect-changes:
    name: detect changes
    runs-on: ubuntu-latest
    outputs:
      core:        ${{ steps.dc.outputs.core }}
      daemon:      ${{ steps.dc.outputs.daemon }}
      cli:         ${{ steps.dc.outputs.cli }}
      runtime:     ${{ steps.dc.outputs.runtime }}
      conformance: ${{ steps.dc.outputs.conformance }}
      xtask:       ${{ steps.dc.outputs.xtask }}
      docs:        ${{ steps.dc.outputs.docs }}
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: detect changes
        id: dc
        run: cargo xtask detect-changes ${{ github.base_ref }}

  lint:
    name: lint + fmt
    runs-on: ubuntu-latest
    needs: [detect-changes]
    if: |
      needs.detect-changes.outputs.core == 'true' ||
      needs.detect-changes.outputs.daemon == 'true' ||
      needs.detect-changes.outputs.cli == 'true' ||
      needs.detect-changes.outputs.runtime == 'true' ||
      needs.detect-changes.outputs.xtask == 'true'
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: cargo xtask lint
        run: cargo xtask lint

  test-unit:
    name: unit tests
    runs-on: ubuntu-latest
    needs: [detect-changes, lint]
    if: |
      needs.detect-changes.outputs.core == 'true' ||
      needs.detect-changes.outputs.daemon == 'true' ||
      needs.detect-changes.outputs.cli == 'true' ||
      needs.detect-changes.outputs.runtime == 'true'
    steps:
      - uses: actions/checkout@v5
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - uses: taiki-e/install-action@v2
        with:
          tool: cargo-nextest
      - name: cargo xtask test-unit
        run: cargo xtask test-unit

  cancel-on-failure:
    name: cancel on failure
    runs-on: ubuntu-latest
    needs: [lint, test-unit]
    if: failure()
    steps:
      - name: cancel workflow run
        run: gh run cancel $RUN_ID --repo $REPO
        env:
          GH_TOKEN: ${{ github.token }}
          RUN_ID: ${{ github.run_id }}
          REPO: ${{ github.repository }}
```

Commit: `git commit -m "ci(pr): add detect-changes job, gate lint and test-unit on changed areas"`

---

### Task 5: Update conformance.yml

**File(s)**: `.github/workflows/conformance.yml`
**Run**: (manual GHA verification after push)

Add a `detect-changes` job and gate the `conformance` job on it. Replace `conformance.yml` with:

```yaml
name: Conformance

on:
  push:
    branches: [develop, next, stable]
  pull_request:
    branches: [develop, next, stable]
  workflow_dispatch:

concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: true

env:
  CARGO_INCREMENTAL: 0

jobs:
  detect-changes:
    name: detect changes
    runs-on: ubuntu-latest
    outputs:
      core:        ${{ steps.dc.outputs.core }}
      runtime:     ${{ steps.dc.outputs.runtime }}
      conformance: ${{ steps.dc.outputs.conformance }}
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: detect changes
        id: dc
        run: |
          if [ "${{ github.event_name }}" = "pull_request" ]; then
            BASE="${{ github.base_ref }}"
          elif [ "${{ github.event_name }}" = "workflow_dispatch" ]; then
            BASE="origin/main"
          else
            BASE="HEAD^"
          fi
          cargo xtask detect-changes "$BASE"

  conformance:
    name: conformance suite (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    needs: [detect-changes]
    if: |
      needs.detect-changes.outputs.core == 'true' ||
      needs.detect-changes.outputs.runtime == 'true' ||
      needs.detect-changes.outputs.conformance == 'true'
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@v5

      - uses: dtolnay/rust-toolchain@stable

      - uses: Swatinem/rust-cache@v2

      - name: run conformance suite
        run: cargo run -p minibox-conformance --bin run-conformance

      - name: upload conformance report
        if: always()
        uses: actions/upload-artifact@v4
        with:
          name: conformance-report-${{ matrix.os }}
          path: artifacts/conformance/
          retention-days: 30

      - name: post summary
        if: always()
        run: |
          REPORT=artifacts/conformance/report.md
          if [ -f "$REPORT" ]; then
            echo "## Conformance Report (${{ matrix.os }})" >> "$GITHUB_STEP_SUMMARY"
            cat "$REPORT" >> "$GITHUB_STEP_SUMMARY"
          fi
```

Commit: `git commit -m "ci(conformance): gate conformance suite on changed areas"`

---

### Task 6: Update merge.yml

**File(s)**: `.github/workflows/merge.yml`
**Run**: (manual GHA verification after push)

Add a `detect-changes` job. Gate `lint` and `test-unit` on it. The heavier jobs (`build-test-archive`,
`test-e2e`, `test-integration`) keep their existing branch guards and are not gated — they
already have `if: contains(fromJSON(...), github.ref)` and run infrequently enough that
skipping them on non-code changes is low value.

Insert this job before `lint`:

```yaml
  detect-changes:
    name: detect changes
    runs-on: ubuntu-latest
    outputs:
      core:    ${{ steps.dc.outputs.core }}
      daemon:  ${{ steps.dc.outputs.daemon }}
      cli:     ${{ steps.dc.outputs.cli }}
      runtime: ${{ steps.dc.outputs.runtime }}
      xtask:   ${{ steps.dc.outputs.xtask }}
    steps:
      - uses: actions/checkout@v5
        with:
          fetch-depth: 0
      - uses: dtolnay/rust-toolchain@stable
      - uses: Swatinem/rust-cache@v2
      - name: detect changes
        id: dc
        run: cargo xtask detect-changes HEAD^
```

Update `lint` job:

```yaml
  lint:
    name: lint + fmt (ubuntu)
    runs-on: ubuntu-latest
    needs: [detect-changes]
    if: |
      needs.detect-changes.outputs.core == 'true' ||
      needs.detect-changes.outputs.daemon == 'true' ||
      needs.detect-changes.outputs.cli == 'true' ||
      needs.detect-changes.outputs.runtime == 'true' ||
      needs.detect-changes.outputs.xtask == 'true'
```

Update `test-unit` job — add `detect-changes` to `needs` and add the same `if` guard as `lint`.

Update `ci-ok` job's `needs` list to include `detect-changes`.

Commit: `git commit -m "ci(merge): add detect-changes job, gate lint and test-unit"`

---

## Out of Scope

- `macbox`/`winbox` job gates — those jobs don't exist in current workflows
- `crux-plugin` area — not in current workflow matrix
- Caching detect-changes results across workflow reruns
- Transitive dependency expansion in `ChangeSet`
