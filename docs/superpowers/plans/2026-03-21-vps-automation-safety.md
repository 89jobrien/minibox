---
status: archived
note: No commit evidence found; sshpass tmpfile pattern not implemented as of 2026-04-23
---

# VPS Automation Safety Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make `xtask bench-vps` safe by (1) requiring explicit `--commit`/`--push` opt-in instead of unconditional auto-push, and (2) replacing `sshpass -p <password>` with `sshpass -f <tmpfile>` to stop leaking the VPS credential in process listings.

**Architecture:** All changes are in `xtask/src/main.rs` — specifically `bench_vps()` and `ssh_sudo_script()`. The git side-effects are gated behind two new CLI flags. The credential is written to a `0600` tempfile, passed to `sshpass -f`, then deleted in a `scopeguard` drop guard so cleanup happens even on early returns. No new dependencies needed — `scopeguard` is already a Cargo ecosystem crate; alternatively a plain RAII wrapper works.

**Tech Stack:** Rust, `sshpass`, `xshell`, `anyhow`, `tempfile` crate (already in workspace as a dev-dep; add to xtask if missing)

---

## File Map

| File                | Change                                                                                                       |
| ------------------- | ------------------------------------------------------------------------------------------------------------ |
| `xtask/src/main.rs` | `bench_vps()` — add `--commit`/`--push` flag parsing; `ssh_sudo_script()` — replace `-p` with `-f <tmpfile>` |
| `xtask/Cargo.toml`  | Add `tempfile` dependency if not already present                                                             |

---

### Task 1: Add `--commit` / `--push` flags to `bench_vps`

**Files:**

- Modify: `xtask/src/main.rs` — `main()` dispatch and `bench_vps()` (line ~379)

- [ ] **Step 1: Write a failing test**

Add to `xtask/src/main.rs` (or a `tests/` file — xtask doesn't have a test suite, so use `#[cfg(test)]` in main.rs):

```rust
#[cfg(test)]
mod bench_vps_args_tests {
    #[test]
    fn bench_vps_args_default_no_commit_no_push() {
        let args: Vec<String> = vec![];
        let (commit, push) = parse_bench_vps_flags(&args);
        assert!(!commit);
        assert!(!push);
    }

    #[test]
    fn bench_vps_args_explicit_flags() {
        let args = vec!["--commit".to_string(), "--push".to_string()];
        let (commit, push) = parse_bench_vps_flags(&args);
        assert!(commit);
        assert!(push);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test -p xtask bench_vps_args 2>&1 | head -20
```

Expected: FAIL — `parse_bench_vps_flags` does not exist.

- [ ] **Step 3: Add `parse_bench_vps_flags`**

Add above `bench_vps()`:

```rust
/// Parse --commit / --push flags from extra args. Returns (commit, push).
fn parse_bench_vps_flags(args: &[String]) -> (bool, bool) {
    let commit = args.iter().any(|a| a == "--commit");
    let push = args.iter().any(|a| a == "--push");
    (commit, push)
}
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test -p xtask bench_vps_args
```

Expected: 2 tests pass.

- [ ] **Step 5: Thread flags through `main()` dispatch**

In `main()`, change the `bench-vps` arm to pass remaining args:

```rust
Some("bench-vps") => {
    let extra: Vec<String> = env::args().skip(2).collect();
    bench_vps(&sh, &extra)
}
```

Update `bench_vps` signature:

```rust
fn bench_vps(sh: &Shell, extra_args: &[String]) -> Result<()> {
```

- [ ] **Step 6: Gate the git block behind the flags**

Inside `bench_vps`, replace the unconditional commit/push block:

```rust
if scp_ok {
    save_bench_results(sh, &tmp_path)?;
    let _ = fs::remove_file(&tmp_path);

    let (do_commit, do_push) = parse_bench_vps_flags(extra_args);

    if do_commit || do_push {
        let sha_short = cmd!(sh, "git rev-parse --short HEAD")
            .read()
            .unwrap_or_default();
        let sha_short = sha_short.trim();
        cmd!(sh, "git add bench/results/bench.jsonl bench/results/latest.json")
            .ignore_status()
            .run()?;
        let msg = format!("bench: vps results @ {sha_short}");
        cmd!(sh, "git commit -m {msg}").ignore_status().run()?;
        eprintln!("✓ bench results committed");
    }

    if do_push {
        cmd!(sh, "git push").run().context("git push failed")?;
        eprintln!("✓ bench results pushed");
    }
} else {
    eprintln!("warning: scp failed — JSON not saved locally");
}
```

- [ ] **Step 7: Build to verify it compiles**

```bash
cargo build -p xtask
```

- [ ] **Step 8: Commit**

```bash
git add xtask/src/main.rs
git commit -m "fix(bench): make bench-vps --commit/--push opt-in, default off"
```

---

### Task 2: Replace `sshpass -p` with `sshpass -f <tmpfile>`

**Files:**

- Modify: `xtask/src/main.rs` — `ssh_sudo_script()` (line ~331) and the two `sshpass` calls in `bench_vps()` for `scp`
- Possibly modify: `xtask/Cargo.toml` — add `tempfile` if not present

- [ ] **Step 1: Check if `tempfile` is available in xtask**

```bash
grep tempfile xtask/Cargo.toml
```

If not present, add it:

```toml
[dependencies]
tempfile = "3"
```

Then `cargo build -p xtask` to confirm it resolves.

- [ ] **Step 2: Write a failing test**

```rust
#[cfg(test)]
mod sshpass_file_tests {
    use super::*;

    #[test]
    fn write_pass_file_creates_readable_file() {
        let (path, _guard) = write_pass_tmpfile("hunter2").unwrap();
        let contents = std::fs::read_to_string(&path).unwrap();
        assert_eq!(contents.trim(), "hunter2");
    }

    #[test]
    fn write_pass_file_has_restricted_permissions() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let (path, _guard) = write_pass_tmpfile("secret").unwrap();
            let meta = std::fs::metadata(&path).unwrap();
            let mode = meta.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "tempfile must be 0600, got {:o}", mode);
        }
    }
}
```

- [ ] **Step 3: Run to verify it fails**

```bash
cargo test -p xtask sshpass_file 2>&1 | head -20
```

Expected: FAIL — `write_pass_tmpfile` does not exist.

- [ ] **Step 4: Implement `write_pass_tmpfile`**

Add near the top of `xtask/src/main.rs` after imports:

```rust
use std::os::unix::fs::PermissionsExt;

/// Write `password` to a 0600 tempfile. Returns (path, NamedTempFile).
/// The caller holds the NamedTempFile to keep it alive; it auto-deletes on drop.
fn write_pass_tmpfile(password: &str) -> Result<(std::path::PathBuf, tempfile::NamedTempFile)> {
    let mut f = tempfile::NamedTempFile::new().context("create password tempfile")?;
    std::fs::set_permissions(f.path(), std::fs::Permissions::from_mode(0o600))
        .context("chmod 0600 password tempfile")?;
    use std::io::Write as _;
    writeln!(f, "{password}").context("write password to tempfile")?;
    f.flush().context("flush password tempfile")?;
    let path = f.path().to_path_buf();
    Ok((path, f))
}
```

- [ ] **Step 5: Run tests to verify they pass**

```bash
cargo test -p xtask sshpass_file
```

Expected: 2 tests pass.

- [ ] **Step 6: Update `ssh_sudo_script` to use `-f`**

`ssh_sudo_script` currently takes `pass: &str` and passes it as `-p`. Change it to write a tempfile and use `-f`:

```rust
fn ssh_sudo_script(pass: &str, script: &str) -> Result<String> {
    let tmpfile = format!("/tmp/xtask-bench-{}.sh", std::process::id());
    let (pass_path, _pass_guard) = write_pass_tmpfile(pass)?;

    // Step 1: upload script
    let write_cmd = format!("cat > '{tmpfile}' && chmod 700 '{tmpfile}'");
    let mut upload = Command::new("sshpass")
        .arg("-f")
        .arg(&pass_path)
        .arg("ssh")
        .args(ssh_opts())
        .arg(VPS_HOST)
        .arg(&write_cmd)
        .stdin(Stdio::piped())
        .spawn()
        .context("failed to spawn sshpass for script upload")?;
    upload
        .stdin
        .take()
        .context("no stdin")?
        .write_all(script.as_bytes())
        .context("failed to write script")?;
    if !upload.wait().context("script upload wait")?.success() {
        bail!("failed to write script to remote");
    }

    // Step 2: run as root; clean up regardless of exit code
    let run_cmd = format!(
        "echo '{}' | sudo -S bash '{tmpfile}'; RC=$?; rm -f '{tmpfile}'; exit $RC",
        pass.replace('\'', "'\\''"),
    );
    let out = Command::new("sshpass")
        .arg("-f")
        .arg(&pass_path)
        .arg("ssh")
        .args(ssh_opts())
        .arg(VPS_HOST)
        .arg(&run_cmd)
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .context("ssh sudo run failed")?;
    if !out.status.success() {
        bail!("remote script exited with status {}", out.status);
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}
```

Note: `_pass_guard` holds the `NamedTempFile` alive for the duration of the function, then drops (deletes) it automatically.

- [ ] **Step 7: Update the `sshpass` call in `bench_vps` for `scp`**

Find the `scp` block in `bench_vps()` and replace:

```rust
let (pass_path, _pass_guard) = write_pass_tmpfile(&vps_pass)?;
let scp_ok = Command::new("sshpass")
    .arg("-f")
    .arg(&pass_path)
    .arg("scp")
    .args(ssh_opts())
    .arg(format!("{VPS_HOST}:/tmp/bench-latest.json"))
    .arg(&tmp_path)
    .status()
    .context("scp failed")?
    .success();
```

- [ ] **Step 8: Build to verify it compiles**

```bash
cargo build -p xtask
```

Expected: clean compile.

- [ ] **Step 9: Run all xtask tests**

```bash
cargo test -p xtask
```

Expected: all pass.

- [ ] **Step 10: Commit**

```bash
git add xtask/src/main.rs xtask/Cargo.toml
git commit -m "fix(security): replace sshpass -p with -f tmpfile to prevent credential leak in ps"
```

---

### Task 3: Update CLAUDE.md quick reference

**Files:**

- Modify: `CLAUDE.md` — bench-vps entry in the quick reference table

- [ ] **Step 1: Find the bench-vps reference**

```bash
grep -n "bench-vps" CLAUDE.md
```

- [ ] **Step 2: Update the description**

Change the `bench-vps` line to note the flags:

```
cargo xtask bench-vps               # run bench on VPS, fetch results (no git side-effects)
cargo xtask bench-vps --commit      # ... and commit results locally
cargo xtask bench-vps --commit --push  # ... and push to remote
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs: update bench-vps usage — --commit/--push flags now required for git ops"
```
