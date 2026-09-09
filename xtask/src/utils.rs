//! Shared xtask path utilities.

use std::{
    env,
    path::{Path, PathBuf},
};

/// Returns the workspace root derived from the xtask Cargo manifest directory.
pub fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask manifest directory should be inside the workspace")
        .to_path_buf()
}

/// Returns the Cargo target directory, respecting `CARGO_TARGET_DIR` if set.
///
/// Relative values are anchored at the workspace root. When the variable is
/// absent, this returns `<workspace-root>/target`.
pub fn cargo_target_dir() -> PathBuf {
    let root = workspace_root();
    match env::var_os("CARGO_TARGET_DIR").map(PathBuf::from) {
        Some(target_dir) if target_dir.is_absolute() => target_dir,
        Some(target_dir) => root.join(target_dir),
        None => root.join("target"),
    }
}

/// Returns the path Cargo uses for a compiled binary.
pub fn cargo_binary_path(
    target_dir: &Path,
    target: Option<&str>,
    profile: &str,
    binary: &str,
) -> PathBuf {
    let mut path = target_dir.to_path_buf();
    if let Some(target) = target {
        path.push(target);
    }
    path.join(profile).join(binary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, sync::Mutex};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        original: Option<OsString>,
    }

    impl EnvGuard {
        fn set(value: Option<&Path>) -> Self {
            let original = env::var_os("CARGO_TARGET_DIR");
            // SAFETY: all tests that mutate CARGO_TARGET_DIR hold ENV_LOCK, and
            // this guard restores the original value before releasing it.
            unsafe {
                match value {
                    Some(value) => env::set_var("CARGO_TARGET_DIR", value),
                    None => env::remove_var("CARGO_TARGET_DIR"),
                }
            }
            Self { original }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: EnvGuard is only constructed while ENV_LOCK is held.
            unsafe {
                match &self.original {
                    Some(value) => env::set_var("CARGO_TARGET_DIR", value),
                    None => env::remove_var("CARGO_TARGET_DIR"),
                }
            }
        }
    }

    #[test]
    fn workspace_root_is_parent_of_xtask_manifest() {
        let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
        assert_eq!(
            workspace_root(),
            manifest_dir.parent().expect("xtask manifest has a parent")
        );
    }

    #[test]
    fn relative_cargo_target_dir_is_anchored_at_workspace_root() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let _env = EnvGuard::set(Some(Path::new("artifacts/target")));

        assert_eq!(
            cargo_target_dir(),
            workspace_root().join("artifacts/target")
        );
    }

    #[test]
    fn absolute_cargo_target_dir_is_unchanged() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let temp = tempfile::tempdir().expect("create temporary target directory");
        let _env = EnvGuard::set(Some(temp.path()));

        assert_eq!(cargo_target_dir(), temp.path());
    }

    #[test]
    fn default_cargo_target_dir_is_under_workspace_root() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let _env = EnvGuard::set(None);

        assert_eq!(cargo_target_dir(), workspace_root().join("target"));
    }

    #[test]
    fn binary_path_includes_optional_target_and_profile() {
        let target_dir = Path::new("/workspace/target");

        assert_eq!(
            cargo_binary_path(
                target_dir,
                Some("x86_64-unknown-linux-musl"),
                "debug",
                "miniboxd"
            ),
            target_dir
                .join("x86_64-unknown-linux-musl")
                .join("debug")
                .join("miniboxd")
        );
        assert_eq!(
            cargo_binary_path(target_dir, None, "release", "mbx"),
            target_dir.join("release").join("mbx")
        );
    }
}
