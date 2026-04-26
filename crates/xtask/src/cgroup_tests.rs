#[cfg(target_os = "linux")]
use anyhow::{Context, Result, bail};
#[cfg(target_os = "linux")]
use std::{
    fs,
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

    // 5. Build the cgroup_tests binary
    eprintln!("=== Building test binary ===");
    let status = Command::new("cargo")
        .args(["build", "-p", "miniboxd", "--test", "cgroup_tests"])
        .current_dir(root)
        .status()
        .context("cargo build")?;
    if !status.success() {
        bail!("cargo build failed");
    }

    // Find the test binary (newest cgroup_tests-* in deps/)
    let test_bin = find_test_binary(root)?;
    eprintln!("Test binary: {}", test_bin.display());

    // 6. Fork a child that moves itself into runner-leaf then execs the test binary.
    //    This replicates the bash subshell trick: sole process in runner-leaf, then exec.
    eprintln!("=== Running cgroup integration tests ===");

    // SAFETY: fork() is called immediately; child writes its PID to cgroup.procs then
    // execs the test binary. The parent waits. No Rust data structures are used after fork
    // in the child path — only raw libc calls and exec.
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => bail!("fork() failed"),
        0 => {
            // Child: move self into runner-leaf, then exec
            let leaf_procs = leaf.join("cgroup.procs");
            let mypid = unsafe { libc::getpid() }.to_string();
            let _ = std::fs::write(&leaf_procs, mypid.as_bytes());

            let bin = std::ffi::CString::new(test_bin.to_str().unwrap()).unwrap();
            let arg0 = bin.clone();
            let threads = std::ffi::CString::new("--test-threads=1").unwrap();
            let nocapture = std::ffi::CString::new("--nocapture").unwrap();
            let argv = [
                arg0.as_ptr(),
                threads.as_ptr(),
                nocapture.as_ptr(),
                std::ptr::null(),
            ];
            unsafe { libc::execv(bin.as_ptr(), argv.as_ptr()) };
            // If execv returns, it failed
            std::process::exit(127);
        }
        child_pid => {
            // Parent: wait for child
            let mut status = 0i32;
            unsafe { libc::waitpid(child_pid, &mut status, 0) };
            let exit_code = if libc::WIFEXITED(status) {
                libc::WEXITSTATUS(status)
            } else {
                1
            };

            // Cleanup
            eprintln!("=== Cleaning up ===");
            for entry in fs::read_dir(&leaf).into_iter().flatten().flatten() {
                if entry.file_type().ok().is_some_and(|t| t.is_dir()) {
                    let _ = fs::remove_dir(entry.path());
                }
            }
            let _ = fs::remove_dir(&leaf);
            let _ = fs::remove_dir(&slice);

            if exit_code != 0 {
                bail!("cgroup tests failed (exit code {exit_code})");
            }
            eprintln!("cgroup tests passed");
            Ok(())
        }
    }
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
    // Build runs without --release, so check debug first
    let deps_debug = root.join("target/debug/deps");
    if deps_debug.exists() {
        if let Ok(bin) = find_in_deps(&deps_debug) {
            return Ok(bin);
        }
    }
    let deps_release = root.join("target/release/deps");
    find_in_deps(&deps_release)
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
