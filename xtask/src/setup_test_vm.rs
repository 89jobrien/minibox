//! setup-test-vm — create and provision a persistent smolvm VM for testing.
//!
//! Creates a named `minibox-ci` machine from `tests/smolfiles/ci-cached.smolfile`,
//! then provisions it with Rust stable toolchain, cargo-nextest, and test deps.
//! Subsequent `test-in-vm` runs detect the machine and use `smolvm machine exec`
//! instead of booting ephemeral VMs.
//!
//! ## Usage
//!
//!   cargo xtask setup-test-vm [--force]
//!
//! `--force` tears down any existing `minibox-ci` machine before recreating.

use anyhow::{Context, Result, bail};
use std::path::Path;
use std::process::Command;

pub const VM_NAME: &str = "minibox-ci";
const SMOLFILE: &str = "tests/smolfiles/ci-cached.smolfile";

/// Provision steps run inside the VM after creation.
const PROVISION_SCRIPT: &str = r#"
set -e

# Install base deps
apk add --no-cache \
    bash coreutils util-linux mount shadow \
    curl gcc musl-dev linux-headers

# Install rustup + stable toolchain
if ! command -v rustup >/dev/null 2>&1; then
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --default-toolchain stable --profile minimal
fi
. "$HOME/.cargo/env"

# Install cargo-nextest (musl binary for aarch64)
if ! command -v cargo-nextest >/dev/null 2>&1; then
    curl -LsSf https://get.nexte.st/latest/linux-arm-musl | tar zxf - -C "$HOME/.cargo/bin"
fi

# Create runtime dirs
mkdir -p /var/lib/minibox /run/minibox

# Smoke test
rustc --version
cargo --version
cargo nextest --version 2>/dev/null || cargo-nextest nextest --version

echo "provision complete"
"#;

pub fn run(workspace_root: &Path, force: bool) -> Result<()> {
    let smolvm = which_smolvm()?;
    let smolfile_path = workspace_root.join(SMOLFILE);

    if !smolfile_path.exists() {
        bail!(
            "smolfile not found: {}\nCreate it first.",
            smolfile_path.display()
        );
    }

    // Check if machine already exists
    let existing = machine_exists(&smolvm, VM_NAME)?;

    if existing && !force {
        println!("machine '{VM_NAME}' already exists; use --force to recreate");
        return Ok(());
    }

    if existing {
        println!("[1/4] tearing down existing '{VM_NAME}' ...");
        let _ = Command::new(&smolvm)
            .args(["machine", "stop", "--name", VM_NAME])
            .status();
        let status = Command::new(&smolvm)
            .args(["machine", "delete", "--force", VM_NAME])
            .status()
            .context("deleting existing machine")?;
        if !status.success() {
            bail!("failed to delete existing machine '{VM_NAME}'");
        }
    } else {
        println!("[1/4] no existing machine to remove");
    }

    // Create from smolfile with explicit workspace mount
    let mount_spec = format!("{}:/mnt/workspace", workspace_root.display());
    println!("[2/4] creating '{VM_NAME}' from {SMOLFILE} ...");
    let status = Command::new(&smolvm)
        .args([
            "machine",
            "create",
            VM_NAME,
            "--smolfile",
            &smolfile_path.to_string_lossy(),
            "-v",
            &mount_spec,
        ])
        .status()
        .context("creating machine")?;
    if !status.success() {
        bail!("smolvm machine create failed");
    }

    // Start
    println!("[3/4] starting '{VM_NAME}' ...");
    let status = Command::new(&smolvm)
        .args(["machine", "start", "--name", VM_NAME])
        .status()
        .context("starting machine")?;
    if !status.success() {
        bail!("smolvm machine start failed");
    }

    // Provision
    println!("[4/4] provisioning (Rust toolchain + deps) ...");
    let status = Command::new(&smolvm)
        .args([
            "machine",
            "exec",
            "--name",
            VM_NAME,
            "--",
            "/bin/sh",
            "-c",
            PROVISION_SCRIPT,
        ])
        .status()
        .context("provisioning machine")?;
    if !status.success() {
        bail!("provisioning failed — check output above");
    }

    println!("'{VM_NAME}' ready. Run `cargo xtask test-in-vm` to use it.");
    Ok(())
}

fn which_smolvm() -> Result<std::path::PathBuf> {
    Command::new("which")
        .arg("smolvm")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| std::path::PathBuf::from(String::from_utf8_lossy(&o.stdout).trim().to_string()))
        .context("smolvm not found on PATH")
}

fn machine_exists(smolvm: &Path, name: &str) -> Result<bool> {
    let output = Command::new(smolvm)
        .args(["machine", "list"])
        .output()
        .context("smolvm machine list")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout.lines().any(|line| line.starts_with(name)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provision_script_is_valid_sh() {
        // Verify the script has set -e and key commands
        assert!(PROVISION_SCRIPT.contains("set -e"));
        assert!(PROVISION_SCRIPT.contains("rustup"));
        assert!(PROVISION_SCRIPT.contains("cargo-nextest"));
        assert!(PROVISION_SCRIPT.contains("provision complete"));
    }
}
