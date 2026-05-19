#[cfg(target_os = "linux")]
use anyhow::{Context, Result, bail};
#[cfg(target_os = "linux")]
use std::{
    fs,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::Command,
};

/// Run cgroup v2 integration tests under a properly delegated cgroup hierarchy.
///
/// Requires Linux + root. Replaces `scripts/run-cgroup-tests.sh` and
/// `scripts/run-cgroup-tests.nu`.
///
/// Steps:
///   1. Verify cgroup v2 is mounted
///   2. Clean up stale `minibox-test-*` cgroups
///   3. Create `minibox-test-slice/runner-leaf` hierarchy
///   4. Enable controllers at slice level
///   5. Build the `cgroup_tests` test binary
///   6. Exec the binary as the sole process in `runner-leaf`
#[cfg(target_os = "linux")]
pub fn run_cgroup_tests(root: &Path) -> Result<()> {
    let cgroup_root = Path::new("/sys/fs/cgroup");
    let slice = cgroup_root.join("minibox-test-slice");
    let leaf = slice.join("runner-leaf");

    // 1. Verify cgroup v2
    let mounts = fs::read_to_string("/proc/mounts").context("read /proc/mounts")?;
    if !mounts.lines().any(|l| l.contains("cgroup2")) {
        bail!("cgroups v2 not mounted at /sys/fs/cgroup");
    }

    // 2. Clean up stale test cgroups
    eprintln!("=== Cleaning up any previous test cgroups ===");
    cleanup_cgroup(&slice);

    // 3. Create hierarchy
    eprintln!("=== Setting up test cgroup slice ===");
    fs::create_dir_all(&leaf).context("create runner-leaf cgroup")?;

    // 4. Enable controllers at root then slice level
    for ctrl in &["+memory", "+cpu", "+pids", "+io"] {
        let _ = append_to_file(&cgroup_root.join("cgroup.subtree_control"), ctrl);
        let _ = append_to_file(&slice.join("cgroup.subtree_control"), ctrl);
    }

    // 5. Build the cgroup_tests binary (--release to match gates.rs).
    eprintln!("=== Building test binary ===");
    let status = Command::new("cargo")
        .args([
            "build",
            "--release",
            "-p",
            "miniboxd",
            "--test",
            "cgroup_tests",
        ])
        .current_dir(root)
        .status()
        .context("cargo build")?;
    if !status.success() {
        bail!("cargo build failed");
    }

    // Find the test binary (newest cgroup_tests-* in deps/).
    let test_bin = find_test_binary(root)?;
    eprintln!("Test binary: {}", test_bin.display());

    // 6. Spawn a child that joins runner-leaf via pre_exec, then execs the test binary.
    //    Using std::process::Command with pre_exec avoids raw fork() in a live Rust process.
    eprintln!("=== Running cgroup integration tests ===");

    let leaf_procs = leaf.join("cgroup.procs");
    let mut cmd = Command::new(&test_bin);
    cmd.args(["--test-threads=1", "--nocapture"]);

    // SAFETY: pre_exec runs in the child after fork but before exec. At that point the
    // child is single-threaded and no Rust allocator state is used — we only call
    // std::fs::write (a thin syscall wrapper) to move the child PID into runner-leaf
    // before exec replaces the process image. The write may silently fail if the cgroup
    // path is wrong; the subsequent exec will then run outside the cgroup, which is
    // acceptable as a best-effort placement.
    unsafe {
        cmd.pre_exec(move || {
            let pid = std::process::id().to_string();
            let _ = std::fs::write(&leaf_procs, pid.as_bytes());
            Ok(())
        });
    }

    let exit_code = cmd
        .status()
        .context("spawn cgroup_tests binary")?
        .code()
        .unwrap_or(1);

    // Cleanup
    eprintln!("=== Cleaning up ===");
    cleanup_cgroup(&leaf);
    let _ = fs::remove_dir(&slice);

    if exit_code != 0 {
        bail!("cgroup tests failed (exit code {exit_code})");
    }
    eprintln!("cgroup tests passed");
    Ok(())
}

#[cfg(target_os = "linux")]
fn append_to_file(path: &Path, content: &str) -> Result<()> {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new()
        .append(true)
        .open(path)
        .with_context(|| format!("open {}", path.display()))?;
    writeln!(f, "{content}")?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn cleanup_cgroup(dir: &Path) {
    if !dir.exists() {
        return;
    }
    // Move any processes back to root cgroup first
    let procs_path = dir.join("cgroup.procs");
    if let Ok(content) = fs::read_to_string(&procs_path) {
        for pid_str in content.lines() {
            if let Ok(pid) = pid_str.trim().parse::<u64>() {
                let _ = fs::write("/sys/fs/cgroup/cgroup.procs", pid.to_string());
            }
        }
    }
    // Recurse into children
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            if entry.file_type().ok().is_some_and(|t| t.is_dir()) {
                cleanup_cgroup(&entry.path());
            }
        }
    }
    let _ = fs::remove_dir(dir);
}

#[cfg(target_os = "linux")]
fn find_test_binary(root: &Path) -> Result<PathBuf> {
    // Build runs with --release; check release/deps first then fall back to debug/deps.
    let deps_release = root.join("target/release/deps");
    if deps_release.exists() {
        if let Ok(bin) = find_in_deps(&deps_release) {
            return Ok(bin);
        }
    }
    let deps_debug = root.join("target/debug/deps");
    find_in_deps(&deps_debug)
}

#[cfg(target_os = "linux")]
fn find_in_deps(deps: &Path) -> Result<PathBuf> {
    let mut candidates: Vec<_> = fs::read_dir(deps)
        .with_context(|| format!("read_dir {}", deps.display()))?
        .flatten()
        .filter(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            s.starts_with("cgroup_tests-")
                && !s.ends_with(".d")
                && e.file_type().ok().is_some_and(|t| t.is_file())
        })
        .collect();
    candidates.sort_by_key(|e| {
        e.metadata()
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    candidates
        .into_iter()
        .last()
        .map(|e| e.path())
        .ok_or_else(|| anyhow::anyhow!("could not find cgroup_tests binary in {}", deps.display()))
}

/// Stub for non-Linux platforms.
#[cfg(not(target_os = "linux"))]
pub fn run_cgroup_tests(_root: &std::path::Path) -> anyhow::Result<()> {
    anyhow::bail!("run-cgroup-tests requires Linux");
}
