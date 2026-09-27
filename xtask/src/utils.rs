//! Shared xtask utilities.

use anyhow::{Context, Result, bail};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

/// Finds the workspace root by walking upward from the process working directory.
pub fn workspace_root() -> Result<PathBuf> {
    let current = env::current_dir().context("read current directory")?;
    workspace_root_from(&current)
}

/// Finds the workspace root by walking upward from `start`.
fn workspace_root_from(start: &Path) -> Result<PathBuf> {
    start
        .ancestors()
        .find(|path| path.join("Cargo.lock").is_file() && path.join("xtask/Cargo.toml").is_file())
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow::anyhow!("workspace root not found from {}", start.display()))
}

/// Returns the Cargo target directory for the current workspace.
pub fn cargo_target_dir() -> Result<PathBuf> {
    cargo_target_dir_for(&workspace_root()?)
}

/// Returns the Cargo target directory for `root`, respecting `CARGO_TARGET_DIR`.
/// Relative overrides are anchored at the workspace root, matching xtask's Cargo commands.
pub fn cargo_target_dir_for(root: &Path) -> Result<PathBuf> {
    let configured = env::var_os("CARGO_TARGET_DIR").map(PathBuf::from);
    Ok(match configured {
        Some(path) if path.is_absolute() => path,
        Some(path) => root.join(path),
        None => root.join("target"),
    })
}

/// Finds the most recently modified test binary matching `prefix` in `deps_dir`.
pub fn find_test_binary(deps_dir: &Path, prefix: &str) -> Option<PathBuf> {
    let artifact_prefix = format!("{prefix}-");
    let mut candidates: Vec<_> = fs::read_dir(deps_dir)
        .ok()?
        .filter_map(std::result::Result::ok)
        .filter(|entry| {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            name.starts_with(&artifact_prefix) && is_executable_file(&entry.path())
        })
        .collect();
    candidates.sort_by_key(|entry| {
        entry
            .metadata()
            .and_then(|metadata| metadata.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH)
    });
    candidates.pop().map(|entry| entry.path())
}

/// Returns an error when `path` does not name an executable build artifact.
pub fn require_binary(path: &Path) -> Result<PathBuf> {
    if is_executable_file(path) {
        Ok(path.to_path_buf())
    } else {
        bail!("executable binary not found: {}", path.display())
    }
}

#[cfg(unix)]
fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn is_executable_file(path: &Path) -> bool {
    path.is_file()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn cargo_target_dir_anchors_relative_override_at_workspace_root() {
        let _guard = ENV_LOCK.lock().expect("environment lock");
        let original = env::var_os("CARGO_TARGET_DIR");
        // SAFETY: this test serializes access to CARGO_TARGET_DIR in this module.
        unsafe { env::set_var("CARGO_TARGET_DIR", "ci-target") };
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask must be inside the workspace")
            .to_path_buf();

        let resolved = cargo_target_dir_for(&root).expect("target directory");

        match original {
            Some(value) => {
                // SAFETY: the environment lock is held until the original value is restored.
                unsafe { env::set_var("CARGO_TARGET_DIR", value) };
            }
            None => {
                // SAFETY: the environment lock is held until the variable is removed.
                unsafe { env::remove_var("CARGO_TARGET_DIR") };
            }
        }
        assert_eq!(resolved, root.join("ci-target"));
    }

    #[test]
    fn workspace_root_is_found_from_nested_directory() {
        let temp = tempfile::tempdir().expect("temporary workspace");
        fs::write(temp.path().join("Cargo.lock"), "").expect("workspace lockfile");
        fs::create_dir_all(temp.path().join("xtask/src")).expect("nested xtask directory");
        fs::write(
            temp.path().join("xtask/Cargo.toml"),
            "[package]\nname='xtask'\n",
        )
        .expect("xtask manifest");

        assert_eq!(
            workspace_root_from(&temp.path().join("xtask/src")).expect("workspace root"),
            temp.path()
        );
    }

    #[test]
    fn binary_discovery_ignores_metadata_files() {
        let temp = tempfile::tempdir().expect("temporary deps directory");
        fs::write(temp.path().join("suite-abc.d"), "metadata").expect("metadata fixture");
        let binary = temp.path().join("suite-abc");
        fs::write(&binary, "binary").expect("binary fixture");
        make_executable(&binary);

        assert_eq!(find_test_binary(temp.path(), "suite"), Some(binary));
    }

    #[test]
    fn binary_discovery_ignores_non_executable_files() {
        let temp = tempfile::tempdir().expect("temporary deps directory");
        let binary = temp.path().join("suite-abc");
        fs::write(&binary, "binary").expect("binary fixture");
        make_executable(&binary);
        std::thread::sleep(std::time::Duration::from_millis(10));
        fs::write(temp.path().join("suite-def"), "not executable").expect("decoy fixture");

        assert_eq!(find_test_binary(temp.path(), "suite"), Some(binary));
    }

    #[cfg(unix)]
    fn make_executable(path: &Path) {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).expect("chmod fixture");
    }

    #[cfg(not(unix))]
    fn make_executable(_path: &Path) {}
}
