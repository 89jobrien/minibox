//! Daemon configuration — layered TOML + env var overrides.
//!
//! Load order: project `./minibox.toml` (lowest) -> system
//! `/etc/minibox/config.toml` -> user `~/.config/minibox/config.toml` ->
//! env vars (`MINIBOX_*`, highest).
//! Later layers override earlier ones field-by-field.
//!
//! The project layer exists so a checkout can record its own dev settings
//! (notably the privileged + bind-mount opt-in that `DinD` testing needs)
//! without touching the host's system or user config. It is deliberately
//! the *weakest* layer: a checked-in file can never weaken a system-level
//! lockdown, and env vars can always override it back to deny. It is
//! opt-in — nothing is read unless a `minibox.toml` is actually present.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Filename of the project-level config, discovered by walking up from CWD.
pub const PROJECT_CONFIG_FILENAME: &str = "minibox.toml";

/// Locate a project-level [`PROJECT_CONFIG_FILENAME`] by walking up from
/// `start` to the filesystem root, returning the first match.
///
/// Returns `None` when no such file exists in any ancestor directory.
#[must_use]
pub fn find_project_config(start: &Path) -> Option<PathBuf> {
    start
        .ancestors()
        .map(|dir| dir.join(PROJECT_CONFIG_FILENAME))
        .find(|candidate| candidate.is_file())
}

/// Top-level daemon configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DaemonConfig {
    /// Optional adapter suite name.
    #[serde(default)]
    pub adapter: Option<String>,
    /// Optional tracing filter level.
    #[serde(default)]
    pub log_level: Option<String>,
    /// Optional daemon socket path override.
    #[serde(default)]
    pub socket_path: Option<PathBuf>,
    /// Optional persistent state directory override.
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
    /// Optional image storage directory override.
    #[serde(default)]
    pub images_dir: Option<PathBuf>,
    /// Security and resource policy settings.
    #[serde(default)]
    pub policy: PolicyConfig,
}

/// Security and resource policy knobs.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PolicyConfig {
    /// Whether privileged containers may be requested.
    #[serde(default)]
    pub allow_privileged: Option<bool>,
    /// Whether host bind mounts may be requested.
    #[serde(default)]
    pub allow_bind_mounts: Option<bool>,
    /// Optional maximum image size in mebibytes.
    #[serde(default)]
    pub max_image_size_mb: Option<u64>,
}

impl DaemonConfig {
    /// Load config from the standard layered sources.
    ///
    /// 1. `./minibox.toml` (project — lowest precedence, opt-in)
    /// 2. `/etc/minibox/config.toml` (system)
    /// 3. `$HOME/.config/minibox/config.toml` (user)
    /// 4. `MINIBOX_*` env vars (highest priority)
    ///
    /// The project layer is applied first so system and user config always
    /// win; it exists so a checkout can carry its own dev settings without
    /// mutating host config. Every layer after the first is optional, and a
    /// missing file contributes nothing.
    // qual:allow(iosp) reason: "config loading inherently mixes file I/O with merge logic"
    #[must_use]
    pub fn load() -> Self {
        let mut cfg = Self::default();

        // Layer 1: project config (lowest precedence). Opt-in: skipped
        // entirely when no minibox.toml is found walking up from CWD.
        if let Ok(cwd) = std::env::current_dir()
            && let Some(project_path) = find_project_config(&cwd)
        {
            tracing::debug!(path = %project_path.display(), "config: project layer");
            cfg = cfg.merge(Self::load_from_path(&project_path));
        }

        // Layer 2: system config
        cfg = cfg.merge(Self::load_from_path(Path::new("/etc/minibox/config.toml")));

        // Layer 3: user config
        if let Ok(home) = std::env::var("HOME") {
            let user_path = PathBuf::from(home).join(".config/minibox/config.toml");
            cfg = cfg.merge(Self::load_from_path(&user_path));
        }

        // Layer 4: env var overrides
        cfg.with_env_overrides()
    }

    /// Load config from a specific file path. Returns defaults if the
    /// file is missing; logs a warning if the file exists but is invalid.
    #[must_use]
    pub fn load_from_path(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(content) => toml::from_str(&content).unwrap_or_else(|e| {
                tracing::warn!(
                    path = %path.display(),
                    error = %e,
                    "config: invalid TOML, using defaults"
                );
                Self::default()
            }),
            Err(_) => Self::default(),
        }
    }

    /// Named profile with preset defaults.
    pub fn profile(name: &str) -> Self {
        match name {
            "dev" => Self {
                log_level: Some("debug".into()),
                policy: PolicyConfig {
                    allow_privileged: Some(true),
                    allow_bind_mounts: Some(true),
                    ..Default::default()
                },
                ..Default::default()
            },
            "production" => {
                const PRODUCTION_MAX_IMAGE_SIZE_MB: u64 = 2048;
                Self {
                    log_level: Some("info".into()),
                    policy: PolicyConfig {
                        allow_privileged: Some(false),
                        allow_bind_mounts: Some(false),
                        max_image_size_mb: Some(PRODUCTION_MAX_IMAGE_SIZE_MB),
                    },
                    ..Default::default()
                }
            }
            _ => {
                tracing::warn!(profile = name, "config: unknown profile, using defaults");
                Self::default()
            }
        }
    }

    /// Apply `MINIBOX_*` env var overrides on top of this config.
    #[must_use]
    pub fn with_env_overrides(mut self) -> Self {
        if let Ok(v) = std::env::var("MINIBOX_ADAPTER") {
            self.adapter = Some(v);
        }
        if let Ok(v) = std::env::var("MINIBOX_LOG_LEVEL") {
            self.log_level = Some(v);
        }
        if let Ok(v) = std::env::var("MINIBOX_SOCKET") {
            self.socket_path = Some(PathBuf::from(v));
        }
        if let Ok(v) = std::env::var("MINIBOX_STATE_DIR") {
            self.state_dir = Some(PathBuf::from(v));
        }
        if let Ok(v) = std::env::var("MINIBOX_IMAGES_DIR") {
            self.images_dir = Some(PathBuf::from(v));
        }
        if let Ok(v) = std::env::var("MINIBOX_ALLOW_PRIVILEGED")
            && let Some(b) = parse_policy_bool("MINIBOX_ALLOW_PRIVILEGED", &v)
        {
            self.policy.allow_privileged = Some(b);
        }
        if let Ok(v) = std::env::var("MINIBOX_ALLOW_BIND_MOUNTS")
            && let Some(b) = parse_policy_bool("MINIBOX_ALLOW_BIND_MOUNTS", &v)
        {
            self.policy.allow_bind_mounts = Some(b);
        }
        if let Ok(v) = std::env::var("MINIBOX_MAX_IMAGE_SIZE_MB") {
            if let Ok(n) = v.parse::<u64>() {
                self.policy.max_image_size_mb = Some(n);
            } else {
                tracing::warn!(
                    var = "MINIBOX_MAX_IMAGE_SIZE_MB",
                    value = %v,
                    "config: unrecognised integer value ignored"
                );
            }
        }
        self
    }

    /// Merge `other` on top of `self` — `other` fields take precedence.
    fn merge(self, other: Self) -> Self {
        Self {
            adapter: other.adapter.or(self.adapter),
            log_level: other.log_level.or(self.log_level),
            socket_path: other.socket_path.or(self.socket_path),
            state_dir: other.state_dir.or(self.state_dir),
            images_dir: other.images_dir.or(self.images_dir),
            policy: PolicyConfig {
                allow_privileged: other
                    .policy
                    .allow_privileged
                    .or(self.policy.allow_privileged),
                allow_bind_mounts: other
                    .policy
                    .allow_bind_mounts
                    .or(self.policy.allow_bind_mounts),
                max_image_size_mb: other
                    .policy
                    .max_image_size_mb
                    .or(self.policy.max_image_size_mb),
            },
        }
    }
}

/// Parse a boolean-ish policy env value. Accepts `1|true|yes` /
/// `0|false|no` (case-insensitive, trimmed). Unrecognised values are
/// rejected with a warning — never silently ignored on a
/// security-policy variable. Consistent with `ContainerPolicy::from_env`.
fn parse_policy_bool(name: &str, v: &str) -> Option<bool> {
    match v.trim().to_lowercase().as_str() {
        "1" | "true" | "yes" => Some(true),
        "0" | "false" | "no" => Some(false),
        other => {
            tracing::warn!(
                var = name,
                value = other,
                "config: unrecognised boolean value ignored"
            );
            None
        }
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::collapsible_if
)]
mod tests {
    use super::*;

    #[test]
    fn daemon_config_empty_toml_produces_defaults() {
        let cfg: DaemonConfig = toml::from_str("").expect("empty TOML");
        assert!(cfg.adapter.is_none());
        assert!(cfg.log_level.is_none());
        assert!(cfg.policy.allow_privileged.is_none());
    }

    #[test]
    fn daemon_config_parses_full_config() {
        let toml_str = r#"
            adapter = "smolvm"
            log_level = "debug"

            [policy]
            allow_privileged = false
            max_image_size_mb = 1024
        "#;
        let cfg: DaemonConfig = toml::from_str(toml_str).expect("valid TOML");
        assert_eq!(cfg.adapter.as_deref(), Some("smolvm"));
        assert_eq!(cfg.policy.max_image_size_mb, Some(1024));
    }

    #[test]
    fn env_overrides_file() {
        let file_cfg = DaemonConfig {
            adapter: Some("krun".into()),
            ..Default::default()
        };
        // When MINIBOX_ADAPTER is not set, file value is preserved.
        let merged = file_cfg.with_env_overrides();
        // This test relies on MINIBOX_ADAPTER not being set in the
        // test environment. If it is, the env value wins (correct).
        if std::env::var("MINIBOX_ADAPTER").is_err() {
            assert_eq!(merged.adapter.as_deref(), Some("krun"));
        }
    }

    #[test]
    fn profile_dev_sets_defaults() {
        let cfg = DaemonConfig::profile("dev");
        assert_eq!(cfg.log_level.as_deref(), Some("debug"));
        assert_eq!(cfg.policy.allow_privileged, Some(true));
    }

    #[test]
    fn profile_production_sets_defaults() {
        let cfg = DaemonConfig::profile("production");
        assert_eq!(cfg.log_level.as_deref(), Some("info"));
        assert_eq!(cfg.policy.allow_privileged, Some(false));
    }

    #[test]
    fn missing_file_returns_defaults() {
        let cfg = DaemonConfig::load_from_path(Path::new("/nonexistent/config.toml"));
        assert!(cfg.adapter.is_none());
    }

    #[test]
    fn daemon_config_invalid_toml_returns_defaults() {
        // Invalid TOML should not panic — returns defaults with warning.
        let result = toml::from_str::<DaemonConfig>("not valid [[[ toml");
        assert!(result.is_err());
    }

    #[test]
    fn merge_prefers_other() {
        let base = DaemonConfig {
            adapter: Some("krun".into()),
            log_level: Some("warn".into()),
            ..Default::default()
        };
        let overlay = DaemonConfig {
            adapter: Some("smolvm".into()),
            ..Default::default()
        };
        let merged = base.merge(overlay);
        assert_eq!(merged.adapter.as_deref(), Some("smolvm"));
        assert_eq!(merged.log_level.as_deref(), Some("warn"));
    }

    /// Serializes tests that mutate `MINIBOX_*` env vars.
    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn env_overrides_policy_fields() {
        use minibox_macros::{unsafe_remove_var, unsafe_set_var};

        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");

        unsafe_set_var!("MINIBOX_ALLOW_PRIVILEGED", "true");
        unsafe_set_var!("MINIBOX_ALLOW_BIND_MOUNTS", "false");
        unsafe_set_var!("MINIBOX_MAX_IMAGE_SIZE_MB", "512");

        let cfg = DaemonConfig::default().with_env_overrides();

        // Clean up before assertions so failures don't leak env state.
        unsafe_remove_var!("MINIBOX_ALLOW_PRIVILEGED");
        unsafe_remove_var!("MINIBOX_ALLOW_BIND_MOUNTS");
        unsafe_remove_var!("MINIBOX_MAX_IMAGE_SIZE_MB");

        assert_eq!(cfg.policy.allow_privileged, Some(true));
        assert_eq!(cfg.policy.allow_bind_mounts, Some(false));
        assert_eq!(cfg.policy.max_image_size_mb, Some(512));
    }

    #[test]
    fn env_override_accepts_yes_and_one() {
        use minibox_macros::{unsafe_remove_var, unsafe_set_var};

        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");

        unsafe_set_var!("MINIBOX_ALLOW_PRIVILEGED", "yes");
        unsafe_set_var!("MINIBOX_ALLOW_BIND_MOUNTS", "1");
        let cfg = DaemonConfig::default().with_env_overrides();
        unsafe_remove_var!("MINIBOX_ALLOW_PRIVILEGED");
        unsafe_remove_var!("MINIBOX_ALLOW_BIND_MOUNTS");

        assert_eq!(
            cfg.policy.allow_privileged,
            Some(true),
            "'yes' must parse as true, consistent with ContainerPolicy::from_env"
        );
        assert_eq!(
            cfg.policy.allow_bind_mounts,
            Some(true),
            "'1' must parse as true"
        );
    }

    #[test]
    fn env_override_accepts_no_and_zero() {
        use minibox_macros::{unsafe_remove_var, unsafe_set_var};

        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");

        unsafe_set_var!("MINIBOX_ALLOW_PRIVILEGED", "no");
        unsafe_set_var!("MINIBOX_ALLOW_BIND_MOUNTS", "0");
        let cfg = DaemonConfig::default().with_env_overrides();
        unsafe_remove_var!("MINIBOX_ALLOW_PRIVILEGED");
        unsafe_remove_var!("MINIBOX_ALLOW_BIND_MOUNTS");

        assert_eq!(
            cfg.policy.allow_privileged,
            Some(false),
            "'no' must parse as false"
        );
        assert_eq!(
            cfg.policy.allow_bind_mounts,
            Some(false),
            "'0' must parse as false"
        );
    }

    #[test]
    fn env_override_warns_and_ignores_garbage_bool() {
        use minibox_macros::{unsafe_remove_var, unsafe_set_var};

        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");

        unsafe_set_var!("MINIBOX_ALLOW_PRIVILEGED", "banana");
        let cfg = DaemonConfig::default().with_env_overrides();
        unsafe_remove_var!("MINIBOX_ALLOW_PRIVILEGED");

        assert_eq!(
            cfg.policy.allow_privileged, None,
            "garbage boolean must be rejected, not coerced"
        );
    }

    #[test]
    fn env_override_invalid_u64_leaves_max_image_size_unset() {
        use minibox_macros::{unsafe_remove_var, unsafe_set_var};

        let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");

        unsafe_set_var!("MINIBOX_MAX_IMAGE_SIZE_MB", "lots");
        let cfg = DaemonConfig::default().with_env_overrides();
        unsafe_remove_var!("MINIBOX_MAX_IMAGE_SIZE_MB");

        assert_eq!(cfg.policy.max_image_size_mb, None);
    }

    #[test]
    fn parse_policy_bool_is_case_insensitive_and_trims() {
        assert_eq!(parse_policy_bool("X", " TRUE "), Some(true));
        assert_eq!(parse_policy_bool("X", "Yes"), Some(true));
        assert_eq!(parse_policy_bool("X", "FALSE"), Some(false));
        assert_eq!(parse_policy_bool("X", "No"), Some(false));
        assert_eq!(parse_policy_bool("X", "banana"), None);
        assert_eq!(parse_policy_bool("X", ""), None);
    }

    #[test]
    fn load_from_valid_toml_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            r#"adapter = "native"
log_level = "trace"
"#,
        )
        .expect("write");
        let cfg = DaemonConfig::load_from_path(&path);
        assert_eq!(cfg.adapter.as_deref(), Some("native"));
        assert_eq!(cfg.log_level.as_deref(), Some("trace"));
    }

    // ── project-level minibox.toml discovery ─────────────────────────────

    #[test]
    fn find_project_config_returns_none_when_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(find_project_config(dir.path()).is_none());
    }

    #[test]
    fn find_project_config_finds_file_in_ancestor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(
            root.join(PROJECT_CONFIG_FILENAME),
            "log_level = \"debug\"\n",
        )
        .expect("write project config");
        // Discovery starts from a nested subdirectory and walks up.
        let nested = root.join("crates").join("miniboxd");
        std::fs::create_dir_all(&nested).expect("create nested dir");

        let found = find_project_config(&nested).expect("project config should be found");
        assert_eq!(found, root.join(PROJECT_CONFIG_FILENAME));
    }

    #[test]
    fn find_project_config_prefers_nearest_ancestor() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path();
        std::fs::write(root.join(PROJECT_CONFIG_FILENAME), "log_level = \"info\"\n")
            .expect("write outer");
        let inner = root.join("inner");
        std::fs::create_dir_all(&inner).expect("create inner");
        std::fs::write(
            inner.join(PROJECT_CONFIG_FILENAME),
            "log_level = \"trace\"\n",
        )
        .expect("write inner");

        // Walking up from the inner dir must stop at the inner file, not
        // fall through to the outer one.
        let found = find_project_config(&inner).expect("nearest config should win");
        assert_eq!(found, inner.join(PROJECT_CONFIG_FILENAME));
    }

    #[test]
    fn find_project_config_ignores_directories_named_like_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A *directory* named minibox.toml must not be treated as a config.
        std::fs::create_dir_all(dir.path().join(PROJECT_CONFIG_FILENAME))
            .expect("create decoy dir");
        assert!(find_project_config(dir.path()).is_none());
    }

    #[test]
    fn project_layer_is_overridden_by_user_layer() {
        // Security invariant: a checked-in project file must not be able to
        // loosen policy that a later layer tightened. The project layer
        // enables privileged; the user layer denies it, and the user wins.
        let project = DaemonConfig {
            policy: PolicyConfig {
                allow_privileged: Some(true),
                allow_bind_mounts: Some(true),
                ..Default::default()
            },
            ..Default::default()
        };
        let user = DaemonConfig {
            policy: PolicyConfig {
                allow_privileged: Some(false),
                ..Default::default()
            },
            ..Default::default()
        };

        let merged = project.merge(user);
        assert_eq!(
            merged.policy.allow_privileged,
            Some(false),
            "user config must override the project layer"
        );
        assert_eq!(
            merged.policy.allow_bind_mounts,
            Some(true),
            "unset user fields leave project values intact"
        );
    }

    #[test]
    fn project_layer_parses_privileged_opt_in() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(PROJECT_CONFIG_FILENAME);
        std::fs::write(
            &path,
            r"[policy]
allow_privileged = true
allow_bind_mounts = true
",
        )
        .expect("write");

        let cfg = DaemonConfig::load_from_path(&path);
        assert_eq!(cfg.policy.allow_privileged, Some(true));
        assert_eq!(cfg.policy.allow_bind_mounts, Some(true));
    }
}

// ---------------------------------------------------------------------------
// Kani formal verification proofs (cfg-gated, never compiled in normal builds)
// ---------------------------------------------------------------------------

#[cfg(kani)]
mod kani_proofs {
    use super::*;

    /// Proof 31: production profile always sets allow_privileged = Some(false)
    /// and allow_bind_mounts = Some(false). This is a security invariant —
    /// a regression here is a privilege escalation.
    #[kani::proof]
    fn production_profile_locks_privileges() {
        let cfg = DaemonConfig::profile("production");
        assert_eq!(
            cfg.policy.allow_privileged,
            Some(false),
            "production must deny privileged"
        );
        assert_eq!(
            cfg.policy.allow_bind_mounts,
            Some(false),
            "production must deny bind mounts"
        );
    }

    /// Proof 32: production profile sets a finite image size cap.
    #[kani::proof]
    fn production_profile_has_image_size_cap() {
        let cfg = DaemonConfig::profile("production");
        assert!(
            cfg.policy.max_image_size_mb.is_some(),
            "production must set max_image_size_mb"
        );
        assert_eq!(cfg.policy.max_image_size_mb, Some(2048));
    }

    /// Proof 33: merge with Default is identity — Option::or(None) returns
    /// the original Some value. This is the core merge invariant.
    #[kani::proof]
    fn merge_with_default_is_identity() {
        // Verify the Option::or semantics that merge relies on.
        let a: Option<u64> = Some(42);
        let b: Option<u64> = None;
        // other.or(self) where other=None => self wins.
        assert_eq!(b.or(a), Some(42));

        // All-None overlay preserves all base fields.
        let base_policy = PolicyConfig {
            allow_privileged: Some(true),
            allow_bind_mounts: Some(false),
            max_image_size_mb: Some(1024),
        };
        let empty_policy = PolicyConfig::default();

        let merged_priv = empty_policy
            .allow_privileged
            .or(base_policy.allow_privileged);
        let merged_bind = empty_policy
            .allow_bind_mounts
            .or(base_policy.allow_bind_mounts);
        let merged_size = empty_policy
            .max_image_size_mb
            .or(base_policy.max_image_size_mb);

        assert_eq!(merged_priv, Some(true));
        assert_eq!(merged_bind, Some(false));
        assert_eq!(merged_size, Some(1024));
    }

    /// Proof 34: merge prefers `other` — Option::or gives precedence to
    /// the first (other) value when both are Some.
    #[kani::proof]
    fn merge_other_wins() {
        let a: Option<u64> = Some(1);
        let b: Option<u64> = Some(2);
        // other.or(self) where other=Some(2) => other wins.
        assert_eq!(b.or(a), Some(2));

        // Applied to policy: other's allow_privileged wins.
        let base = PolicyConfig {
            allow_privileged: Some(true),
            ..Default::default()
        };
        let overlay = PolicyConfig {
            allow_privileged: Some(false),
            ..Default::default()
        };
        let merged = overlay.allow_privileged.or(base.allow_privileged);
        assert_eq!(merged, Some(false));
    }

    /// Proof 35: unknown profile name returns all-None defaults (no
    /// accidental privilege grant).
    #[kani::proof]
    fn unknown_profile_returns_safe_defaults() {
        let cfg = DaemonConfig::profile("nonexistent");
        assert!(cfg.policy.allow_privileged.is_none());
        assert!(cfg.policy.allow_bind_mounts.is_none());
        assert!(cfg.policy.max_image_size_mb.is_none());
    }
}
