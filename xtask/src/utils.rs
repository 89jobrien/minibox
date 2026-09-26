//! Shared xtask path utilities.
//!
//! Single source of truth for locating the workspace root, the Cargo target
//! directory, and built artifacts. Every xtask module resolves paths through
//! these helpers so local development, CI, and the VM test runners agree.

use std::{
    env,
    path::{Path, PathBuf},
};

/// A concrete Cargo build profile, i.e. a `debug` or `release` output directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Unoptimised development builds, under `target/debug`.
    Debug,
    /// Optimised builds, under `target/release`.
    Release,
}

impl Profile {
    /// The directory name Cargo uses for this profile.
    const fn directory(self) -> &'static str {
        match self {
            Self::Debug => "debug",
            Self::Release => "release",
        }
    }
}

/// Search order that prefers optimised builds, falling back to `debug`.
///
/// Pass this to [`profile_dirs`] / [`deps_dirs`] to probe for artifacts
/// without hard-coding a profile at the call site.
pub const PREFER_RELEASE: [Profile; 2] = [Profile::Release, Profile::Debug];

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

/// Returns the profile output directory for a specific build profile.
pub fn profile_dir(target_dir: &Path, target: Option<&str>, profile: Profile) -> PathBuf {
    let mut path = target_dir.to_path_buf();
    if let Some(target) = target {
        path.push(target);
    }
    path.join(profile.directory())
}

/// Returns the profile output directories to search, in the given priority order.
pub fn profile_dirs(target_dir: &Path, target: Option<&str>, profiles: &[Profile]) -> Vec<PathBuf> {
    profiles
        .iter()
        .map(|profile| profile_dir(target_dir, target, *profile))
        .collect()
}

/// Returns the `deps` directory holding test harness artifacts for a profile.
pub fn deps_dir(target_dir: &Path, target: Option<&str>, profile: Profile) -> PathBuf {
    profile_dir(target_dir, target, profile).join("deps")
}

/// Returns the `deps` directories to search, in the given priority order.
pub fn deps_dirs(target_dir: &Path, target: Option<&str>, profiles: &[Profile]) -> Vec<PathBuf> {
    profiles
        .iter()
        .map(|profile| deps_dir(target_dir, target, *profile))
        .collect()
}

/// Returns the path Cargo uses for a compiled binary.
pub fn cargo_binary_path(
    target_dir: &Path,
    target: Option<&str>,
    profile: Profile,
    binary: &str,
) -> PathBuf {
    profile_dir(target_dir, target, profile).join(binary)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsString, fs, sync::Mutex};

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

    /// Creates an empty artifact at `relative` under `root`, returning its path.
    fn write_artifact(root: &Path, relative: &str) -> PathBuf {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("artifact has a parent dir"))
            .expect("create artifact parent dir");
        fs::write(&path, b"artifact").expect("write artifact");
        path
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
    fn target_dir_is_identical_from_workspace_root_and_nested_subdirectory() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let _env = EnvGuard::set(None);

        let nested = workspace_root().join("crates").join("minibox").join("src");
        assert!(
            nested.is_dir(),
            "expected nested subdir {}",
            nested.display()
        );

        let from_root = cargo_target_dir();
        let previous = env::current_dir().expect("resolve process cwd");
        env::set_current_dir(&nested).expect("enter nested subdirectory");
        let from_nested = cargo_target_dir();
        env::set_current_dir(&previous).expect("restore process cwd");

        assert_eq!(
            from_nested, from_root,
            "target dir must not depend on the process working directory"
        );
        assert_eq!(from_nested, workspace_root().join("target"));
    }

    #[test]
    fn binary_path_includes_optional_target_and_profile() {
        let target_dir = Path::new("/workspace/target");

        assert_eq!(
            cargo_binary_path(
                target_dir,
                Some("x86_64-unknown-linux-musl"),
                Profile::Debug,
                "miniboxd"
            ),
            target_dir
                .join("x86_64-unknown-linux-musl")
                .join("debug")
                .join("miniboxd")
        );
        assert_eq!(
            cargo_binary_path(target_dir, None, Profile::Release, "mbx"),
            target_dir.join("release").join("mbx")
        );
    }

    #[test]
    fn profile_directory_names_match_cargo() {
        assert_eq!(Profile::Debug.directory(), "debug");
        assert_eq!(Profile::Release.directory(), "release");
    }

    #[test]
    fn profile_dirs_prefer_release_before_debug() {
        let target_dir = Path::new("/workspace/target");

        assert_eq!(
            profile_dirs(target_dir, None, &PREFER_RELEASE),
            vec![
                PathBuf::from("/workspace/target/release"),
                PathBuf::from("/workspace/target/debug"),
            ]
        );
    }

    #[test]
    fn profile_dirs_with_pinned_profile_return_one_candidate() {
        let target_dir = Path::new("/workspace/target");

        assert_eq!(
            profile_dirs(target_dir, Some("aarch64-linux-musl"), &[Profile::Debug]),
            vec![PathBuf::from("/workspace/target/aarch64-linux-musl/debug")]
        );
    }

    #[test]
    fn deps_dirs_prefer_release_before_debug() {
        let target_dir = Path::new("/workspace/target");

        assert_eq!(
            deps_dirs(
                target_dir,
                Some("x86_64-unknown-linux-musl"),
                &PREFER_RELEASE
            ),
            vec![
                PathBuf::from("/workspace/target/x86_64-unknown-linux-musl/release/deps"),
                PathBuf::from("/workspace/target/x86_64-unknown-linux-musl/debug/deps"),
            ]
        );
    }

    #[test]
    fn deps_dir_omits_the_target_triple_when_absent() {
        assert_eq!(
            deps_dir(Path::new("/workspace/target"), None, Profile::Debug),
            PathBuf::from("/workspace/target/debug/deps")
        );
    }

    #[test]
    fn profile_search_order_prefers_release_then_debug() {
        let temp = tempfile::tempdir().expect("create temporary target directory");
        let release = write_artifact(temp.path(), "release/mbx");
        let _debug = write_artifact(temp.path(), "debug/mbx");

        // Mirrors how callers probe an ordered search list: the release entry
        // is checked first, so it wins even when debug also holds the binary.
        let ordered: Vec<PathBuf> = PREFER_RELEASE
            .iter()
            .map(|profile| cargo_binary_path(temp.path(), None, *profile, "mbx"))
            .collect();

        assert_eq!(ordered.first(), Some(&release));
        assert!(ordered.iter().all(|path| path.is_file()));
    }

    #[test]
    fn profile_search_order_skips_missing_profiles() {
        let temp = tempfile::tempdir().expect("create temporary target directory");
        let debug = write_artifact(temp.path(), "debug/mbx");

        let located = PREFER_RELEASE
            .iter()
            .map(|profile| cargo_binary_path(temp.path(), None, *profile, "mbx"))
            .find(|path| path.is_file());

        assert_eq!(
            located,
            Some(debug),
            "the debug fallback must be used when release was never built"
        );
    }

    #[test]
    fn profile_search_order_locates_nothing_when_no_artifact_exists() {
        let temp = tempfile::tempdir().expect("create temporary target directory");

        let located = PREFER_RELEASE
            .iter()
            .map(|profile| cargo_binary_path(temp.path(), None, *profile, "mbx"))
            .find(|path| path.is_file());

        assert_eq!(located, None);
    }

    #[test]
    fn profile_search_order_honours_a_pinned_profile() {
        let temp = tempfile::tempdir().expect("create temporary target directory");
        let _release = write_artifact(temp.path(), "release/mbx");

        let located = [Profile::Debug]
            .iter()
            .map(|profile| cargo_binary_path(temp.path(), None, *profile, "mbx"))
            .find(|path| path.is_file());

        assert_eq!(
            located, None,
            "a pinned debug lookup must not fall through to release"
        );
    }

    #[test]
    fn profile_search_order_ignores_a_directory_named_like_a_binary() {
        let temp = tempfile::tempdir().expect("create temporary target directory");
        fs::create_dir_all(temp.path().join("release/mbx")).expect("create decoy directory");

        let located = PREFER_RELEASE
            .iter()
            .map(|profile| cargo_binary_path(temp.path(), None, *profile, "mbx"))
            .find(|path| path.is_file());

        assert_eq!(located, None, "only regular files count as built artifacts");
    }

    #[test]
    fn profile_search_order_includes_the_target_triple_directory() {
        let temp = tempfile::tempdir().expect("create temporary target directory");
        let expected = write_artifact(temp.path(), "x86_64-unknown-linux-musl/debug/miniboxd");

        let located = [Profile::Debug]
            .iter()
            .map(|profile| {
                cargo_binary_path(
                    temp.path(),
                    Some("x86_64-unknown-linux-musl"),
                    *profile,
                    "miniboxd",
                )
            })
            .find(|path| path.is_file());

        assert_eq!(located, Some(expected));
    }

    #[test]
    fn deps_dirs_honour_a_relative_cargo_target_dir() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());

        // A relative CARGO_TARGET_DIR must resolve against the workspace root,
        // not the process working directory.
        let temp = tempfile::Builder::new()
            .prefix("xtask-relative-target-")
            .tempdir_in(workspace_root())
            .expect("create target dir under the workspace root");
        let relative = temp
            .path()
            .strip_prefix(workspace_root())
            .expect("temp dir lives under the workspace root");
        let _env = EnvGuard::set(Some(relative));

        let expected = write_artifact(temp.path(), "debug/mbx");
        let located = cargo_target_dir();
        let located = [Profile::Debug]
            .iter()
            .map(|profile| cargo_binary_path(&located, None, *profile, "mbx"))
            .find(|path| path.is_file());

        assert_eq!(located, Some(expected));
    }

    #[test]
    fn deps_dir_honours_a_relative_cargo_target_dir() {
        let _lock = ENV_LOCK.lock().unwrap_or_else(|poison| poison.into_inner());
        let _env = EnvGuard::set(Some(Path::new("artifacts/nested/target")));

        assert_eq!(
            deps_dir(&cargo_target_dir(), None, Profile::Debug),
            workspace_root()
                .join("artifacts/nested/target")
                .join("debug")
                .join("deps")
        );
    }
}
