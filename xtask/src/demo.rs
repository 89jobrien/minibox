//! `cargo xtask demo [--adapter <name>]` — short end-to-end demonstration.
//!
//! Sets `MINIBOX_ADAPTER` and runs a pull + run sequence against the named
//! adapter. The command is advisory: it exits 0 even when the container
//! commands fail (they require Linux or a running VM). On macOS the smolvm
//! and krun adapters require the VM kernel image to be present; the commands
//! are attempted and failures are printed with an informative note.

use anyhow::Result;
use std::path::Path;
use std::process::Command;
use xshell::Shell;

/// Locate the `mbx` binary: prefer one already on PATH, then fall back to the
/// workspace debug build so the demo works straight after `cargo build`.
fn find_mbx(root: &Path) -> Option<std::path::PathBuf> {
    // Check PATH first.
    if let Ok(out) = Command::new("which").arg("mbx").output()
        && out.status.success()
    {
        let p = std::path::PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
        if p.exists() {
            return Some(p);
        }
    }
    // Fall back to workspace debug build.
    let built = root.join("target/debug/mbx");
    if built.exists() {
        return Some(built);
    }
    None
}

pub fn demo(_sh: &Shell, root: &Path, adapter: &str) -> Result<()> {
    eprintln!("=== minibox demo ===");
    eprintln!("adapter: {adapter}");

    if cfg!(target_os = "macos") {
        eprintln!(
            "note: on macOS the {adapter} adapter requires a VM kernel image \
             (~/.minibox/vm). Commands will run but may fail gracefully if the \
             image is not present."
        );
    }

    let mbx = match find_mbx(root) {
        Some(p) => p,
        None => {
            eprintln!(
                "note: mbx binary not found on PATH or in target/debug/. \
                 Run `cargo build -p mbx` first, or install mbx."
            );
            eprintln!("=== demo complete (skipped — mbx not found) ===");
            return Ok(());
        }
    };

    eprintln!();
    eprintln!("$ mbx pull alpine:latest");
    let pull_status = Command::new(&mbx)
        .arg("pull")
        .arg("alpine:latest")
        .env("MINIBOX_ADAPTER", adapter)
        .status();
    match pull_status {
        Ok(s) if s.success() => eprintln!("pull: ok"),
        Ok(s) => eprintln!("pull: exited with {s} (non-fatal in demo)"),
        Err(e) => eprintln!("pull: could not run mbx: {e} (non-fatal in demo)"),
    }

    eprintln!();
    eprintln!("$ mbx run --rm alpine:latest echo \"hello from minibox\"");
    let run_status = Command::new(&mbx)
        .args(["run", "--rm", "alpine:latest", "echo", "hello from minibox"])
        .env("MINIBOX_ADAPTER", adapter)
        .status();
    match run_status {
        Ok(s) if s.success() => eprintln!("run: ok"),
        Ok(s) => eprintln!("run: exited with {s} (non-fatal in demo)"),
        Err(e) => eprintln!("run: could not run mbx: {e} (non-fatal in demo)"),
    }

    eprintln!();
    eprintln!("=== demo complete ===");
    Ok(())
}
