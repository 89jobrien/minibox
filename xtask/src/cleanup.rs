use anyhow::Result;
use std::{fs, path::Path};
use xshell::{Shell, cmd};

/// Remove non-critical build outputs (preserves incremental cache and registry)
pub fn clean_artifacts(sh: &Shell) -> Result<()> {
    for dir in &["target/debug", "target/release"] {
        let p = Path::new(dir);
        if p.exists() {
            for entry in fs::read_dir(p).into_iter().flatten().flatten() {
                if entry.file_type().ok().is_some_and(|t| t.is_file()) {
                    fs::remove_file(entry.path()).ok();
                }
            }
        }
    }

    for dir in &["target/debug/deps", "target/release/deps"] {
        let p = Path::new(dir);
        if p.exists() {
            for entry in fs::read_dir(p).into_iter().flatten().flatten() {
                let path = entry.path();
                let keep = path.extension().is_some_and(|e| e == "d");
                if !keep && entry.file_type().ok().is_some_and(|t| t.is_file()) {
                    fs::remove_file(&path).ok();
                }
            }
        }
    }

    // Remove .dSYM bundles (macOS debug info directories)
    let _ = sh
        .cmd("find")
        .args([
            "target", "-type", "d", "-name", "*.dSYM", "-exec", "rm", "-rf", "{}", "+",
        ])
        .ignore_status()
        .run();

    eprintln!("artifacts cleaned");
    Ok(())
}

/// Kill orphan processes, unmount overlays, remove test cgroups, clean temp dirs
pub fn nuke_test_state(sh: &Shell) -> Result<()> {
    cmd!(sh, "pkill -f miniboxd.*minibox-test")
        .ignore_status()
        .run()?;
    cmd!(
        sh,
        "bash -c \"mount | grep minibox-test | awk '{print $3}' | xargs -r umount\""
    )
    .ignore_status()
    .run()?;
    cmd!(sh, "bash -c \"systemctl list-units --type=scope --no-legend 2>/dev/null | grep minibox-test | awk '{print $1}' | xargs -r systemctl stop\"")
        .ignore_status()
        .run()?;
    let _ = sh
        .cmd("find")
        .args([
            "/sys/fs/cgroup",
            "-name",
            "minibox-test-*",
            "-type",
            "d",
            "-exec",
            "rmdir",
            "{}",
            "+",
        ])
        .ignore_status()
        .run();
    cmd!(sh, "rm -rf /tmp/minibox-test-*")
        .ignore_status()
        .run()?;
    eprintln!("test state cleaned");
    Ok(())
}
