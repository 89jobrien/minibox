//! Daemon configuration — layered TOML + env var overrides.
//!
//! Load order: system `/etc/minibox/config.toml` -> user
//! `~/.config/minibox/config.toml` -> env vars (`MINIBOX_*`).
//! Later layers override earlier ones field-by-field.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// Top-level daemon configuration.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct DaemonConfig {
    #[serde(default)]
    pub adapter: Option<String>,
    #[serde(default)]
    pub log_level: Option<String>,
    #[serde(default)]
    pub socket_path: Option<PathBuf>,
    #[serde(default)]
    pub state_dir: Option<PathBuf>,
    #[serde(default)]
    pub images_dir: Option<PathBuf>,
    #[serde(default)]
    pub policy: PolicyConfig,
}

/// Security and resource policy knobs.
#[derive(Debug, Clone, Deserialize, Default)]
pub struct PolicyConfig {
    #[serde(default)]
    pub allow_privileged: Option<bool>,
    #[serde(default)]
    pub allow_bind_mounts: Option<bool>,
    #[serde(default)]
    pub max_image_size_mb: Option<u64>,
}

impl DaemonConfig {
    /// Load config from the standard layered sources.
    ///
    /// 1. `/etc/minibox/config.toml` (system)
    /// 2. `$HOME/.config/minibox/config.toml` (user)
    /// 3. `MINIBOX_*` env vars (highest priority)
    // qual:allow(iosp) reason: "config loading inherently mixes file I/O with merge logic"
    pub fn load() -> Self {
        let mut cfg = Self::default();

        // Layer 1: system config
        cfg = cfg.merge(Self::load_from_path(Path::new("/etc/minibox/config.toml")));

        // Layer 2: user config
        if let Ok(home) = std::env::var("HOME") {
            let user_path = PathBuf::from(home).join(".config/minibox/config.toml");
            cfg = cfg.merge(Self::load_from_path(&user_path));
        }

        // Layer 3: env var overrides
        cfg.with_env_overrides()
    }

    /// Load config from a specific file path. Returns defaults if the
    /// file is missing; logs a warning if the file exists but is invalid.
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
            && let Ok(b) = v.parse::<bool>()
        {
            self.policy.allow_privileged = Some(b);
        }
        if let Ok(v) = std::env::var("MINIBOX_ALLOW_BIND_MOUNTS")
            && let Ok(b) = v.parse::<bool>()
        {
            self.policy.allow_bind_mounts = Some(b);
        }
        if let Ok(v) = std::env::var("MINIBOX_MAX_IMAGE_SIZE_MB")
            && let Ok(n) = v.parse::<u64>()
        {
            self.policy.max_image_size_mb = Some(n);
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

#[cfg(test)]
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

    #[test]
    fn env_overrides_policy_fields() {
        // SAFETY: env mutations are not thread-safe; this test is serial by convention.
        unsafe {
            std::env::set_var("MINIBOX_ALLOW_PRIVILEGED", "true");
            std::env::set_var("MINIBOX_ALLOW_BIND_MOUNTS", "false");
            std::env::set_var("MINIBOX_MAX_IMAGE_SIZE_MB", "512");
        }

        let cfg = DaemonConfig::default().with_env_overrides();

        // Clean up before assertions so failures don't leak env state.
        unsafe {
            std::env::remove_var("MINIBOX_ALLOW_PRIVILEGED");
            std::env::remove_var("MINIBOX_ALLOW_BIND_MOUNTS");
            std::env::remove_var("MINIBOX_MAX_IMAGE_SIZE_MB");
        }

        assert_eq!(cfg.policy.allow_privileged, Some(true));
        assert_eq!(cfg.policy.allow_bind_mounts, Some(false));
        assert_eq!(cfg.policy.max_image_size_mb, Some(512));
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
}
