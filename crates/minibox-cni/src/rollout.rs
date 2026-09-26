//! The operational rollout contract for the CNI provider, encoded as a
//! validated struct rather than prose.
//!
//! [`CniRollout`] is the single place that records *where* the packaged
//! plugin binary is expected on disk, *which* `miniboxd` feature gates the
//! provider, and *what* environment the daemon reads. `miniboxd` reads the
//! same three variables with the same defaults, so drift between this
//! struct and the daemon is a test failure, not a production surprise.
//!
//! [`CniRollout::preflight`] turns the contract into a check: the
//! `.conflist`, every plugin binary in the chain, and the spec version each
//! binary advertises must all agree with [`crate::version::CNI_SPEC_VERSION`].

use crate::config::NetworkConfigList;
use crate::error::CniError;
use crate::plugin::PluginEnvironment;
use crate::version::validate_spec_version;
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

/// Variable selecting the network provider implementation.
pub const ENV_MINIBOX_NETWORK_MODE: &str = "MINIBOX_NETWORK_MODE";

/// Variable holding the platform-separated CNI plugin search path.
pub const ENV_MINIBOX_CNI_PATH: &str = "MINIBOX_CNI_PATH";

/// Variable holding the directory containing the `.conflist`.
pub const ENV_MINIBOX_CNI_CONFIG_DIR: &str = "MINIBOX_CNI_CONFIG_DIR";

/// The `miniboxd` cargo feature that gates this provider.
pub const DAEMON_FEATURE: &str = "cni";

/// The `MINIBOX_NETWORK_MODE` value that selects [`crate::CniNetworkProvider`].
pub const NETWORK_MODE: &str = "bridge";

/// Default `MINIBOX_CNI_PATH`, matching `miniboxd`.
pub const DEFAULT_CNI_PATH: &str = "/opt/cni/bin";

/// Default `MINIBOX_CNI_CONFIG_DIR`, matching `miniboxd`.
pub const DEFAULT_CNI_CONFIG_DIR: &str = "/etc/cni/net.d";

/// The one `.conflist` filename the provider reads, matching `miniboxd`.
pub const CONFLIST_FILE_NAME: &str = "10-minibox.conflist";

/// The packaged bridge plugin's binary name, as installed on `CNI_PATH`.
pub const BRIDGE_PLUGIN_NAME: &str = "minibox-bridge";

/// Port: inspect the plugin binaries installed on `CNI_PATH`.
///
/// The production adapter stats the candidate paths and spawns the winner
/// with `CNI_COMMAND=VERSION`; tests substitute an in-memory table. Keeping
/// both halves behind the port is what lets [`CniRollout::preflight`] stay
/// a pure function — it never stats a path itself.
pub trait PluginProbe {
    /// Whether `candidate` is an installed, executable plugin binary.
    fn is_installed(&self, candidate: &Path) -> bool;

    /// Return the spec version the installed `binary` advertises.
    ///
    /// # Errors
    ///
    /// Returns a [`CniError`] when the binary is missing, is not
    /// executable, or does not answer with a decodable `VERSION` reply.
    fn advertised_spec_version(&self, binary: &Path) -> Result<String, CniError>;
}

/// What one plugin in the chain reported during preflight.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PluginProbeReport {
    /// The plugin's `type` as written in the `.conflist`.
    pub plugin_type: String,
    /// Where the binary was found on `CNI_PATH`.
    pub path: PathBuf,
    /// The spec version the binary advertised.
    pub advertised_spec_version: String,
}

/// The outcome of a successful [`CniRollout::preflight`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CniPreflightReport {
    /// Network name from the `.conflist`.
    pub network_name: String,
    /// The `.conflist`'s validated spec version.
    pub conflist_spec_version: String,
    /// One entry per plugin in the chain, in chain order.
    pub plugins: Vec<PluginProbeReport>,
}

/// The validated rollout expectations for the CNI-backed provider.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CniRollout {
    /// The `MINIBOX_NETWORK_MODE` value that selects this provider.
    pub network_mode: String,
    /// Platform-separated plugin search path, in order.
    pub cni_path: Vec<PathBuf>,
    /// Directory containing the `.conflist`.
    pub config_dir: PathBuf,
    /// The `miniboxd` cargo feature that gates this provider.
    pub daemon_feature: String,
    /// The packaged bridge plugin's binary name.
    pub bridge_plugin: String,
}

impl CniRollout {
    /// The out-of-the-box expectations, matching `miniboxd`'s defaults.
    #[must_use]
    pub fn defaults() -> Self {
        Self {
            network_mode: NETWORK_MODE.to_string(),
            cni_path: vec![PathBuf::from(DEFAULT_CNI_PATH)],
            config_dir: PathBuf::from(DEFAULT_CNI_CONFIG_DIR),
            daemon_feature: DAEMON_FEATURE.to_string(),
            bridge_plugin: BRIDGE_PLUGIN_NAME.to_string(),
        }
    }

    /// Read the rollout from `env`, falling back to [`CniRollout::defaults`].
    ///
    /// An override that is set but empty is treated as unset, which is how
    /// an operator clearing a variable in a unit file expects it to behave.
    #[must_use]
    pub fn from_environment(env: &dyn PluginEnvironment) -> Self {
        let defaults = Self::defaults();
        Self {
            network_mode: env
                .get(ENV_MINIBOX_NETWORK_MODE)
                .unwrap_or(defaults.network_mode),
            cni_path: env
                .get(ENV_MINIBOX_CNI_PATH)
                .map_or(defaults.cni_path, |raw| {
                    std::env::split_paths(&raw).collect()
                }),
            config_dir: env
                .get(ENV_MINIBOX_CNI_CONFIG_DIR)
                .map_or(defaults.config_dir, PathBuf::from),
            daemon_feature: defaults.daemon_feature,
            bridge_plugin: defaults.bridge_plugin,
        }
    }

    /// The one `.conflist` this rollout reads.
    #[must_use]
    pub fn conflist_path(&self) -> PathBuf {
        self.config_dir.join(CONFLIST_FILE_NAME)
    }

    /// The `(cni_path, config_dir)` pair `miniboxd` hands to
    /// [`crate::CniNetworkProvider::new`].
    #[must_use]
    pub fn provider_arguments(&self) -> (Vec<PathBuf>, PathBuf) {
        (self.cni_path.clone(), self.config_dir.clone())
    }

    /// A one-line, operator-facing description of where the plugin binary
    /// and the `.conflist` are expected.
    #[must_use]
    pub fn install_hint(&self) -> String {
        format!(
            "build miniboxd with the `{daemon_feature}` feature, install `{plugin}` into one of \
             {path}, and place {conflist}; then set {mode_var}={mode}.",
            daemon_feature = self.daemon_feature,
            plugin = self.bridge_plugin,
            path = self
                .cni_path
                .iter()
                .map(|dir| dir.display().to_string())
                .collect::<Vec<_>>()
                .join(", "),
            conflist = self.conflist_path().display(),
            mode_var = ENV_MINIBOX_NETWORK_MODE,
            mode = self.network_mode,
        )
    }

    /// Check that `conflist` and every plugin it names satisfy the contract.
    ///
    /// Three things must agree with [`crate::version::CNI_SPEC_VERSION`]:
    /// the `.conflist`'s own `cniVersion`, the presence of each plugin
    /// binary on [`CniRollout::cni_path`], and the version each binary
    /// advertises. Nothing here touches the filesystem — the `.conflist` is
    /// passed in already loaded, and both the stat and the `VERSION` spawn
    /// go through `probe`.
    ///
    /// # Errors
    ///
    /// Returns [`CniError::UnsupportedSpecVersion`] if the `.conflist` or an
    /// advertised plugin version is outside
    /// [`crate::version::SUPPORTED_SPEC_VERSIONS`], or
    /// [`CniError::PluginNotFound`] if a plugin binary is absent from
    /// `cni_path`.
    pub fn preflight(
        &self,
        conflist: &NetworkConfigList,
        probe: &dyn PluginProbe,
    ) -> Result<CniPreflightReport, CniError> {
        let conflist_spec_version = conflist.checked_spec_version()?.to_string();
        let mut plugins = Vec::with_capacity(conflist.plugins.len());
        for plugin in &conflist.plugins {
            let path = self.resolve(&plugin.plugin_type, probe)?;
            let advertised = probe.advertised_spec_version(&path)?;
            validate_spec_version(&advertised)?;
            plugins.push(PluginProbeReport {
                plugin_type: plugin.plugin_type.clone(),
                path,
                advertised_spec_version: advertised,
            });
        }
        Ok(CniPreflightReport {
            network_name: conflist.name.clone(),
            conflist_spec_version,
            plugins,
        })
    }

    /// Resolve `plugin_type` to the first installed binary across
    /// [`CniRollout::cni_path`], applying the same single-component
    /// traversal guard the exec path uses.
    fn resolve(&self, plugin_type: &str, probe: &dyn PluginProbe) -> Result<PathBuf, CniError> {
        let name = Path::new(plugin_type);
        if !matches!(
            name.components().collect::<Vec<_>>().as_slice(),
            [Component::Normal(_)]
        ) {
            return Err(CniError::InvalidPluginType {
                plugin: plugin_type.to_string(),
            });
        }
        for dir in &self.cni_path {
            let candidate = dir.join(name);
            if probe.is_installed(&candidate) {
                return Ok(candidate);
            }
        }
        Err(CniError::PluginNotFound {
            plugin: plugin_type.to_string(),
            searched: self.cni_path.clone(),
        })
    }
}

/// The rollout a fresh install starts from, for operators and docs.
#[must_use]
pub fn default_rollout() -> CniRollout {
    CniRollout::defaults()
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::config::PluginConfig;
    use std::collections::{BTreeMap, BTreeSet};

    /// In-memory [`PluginEnvironment`] double.
    struct FakeEnv(BTreeMap<String, String>);

    impl FakeEnv {
        fn new(pairs: &[(&str, &str)]) -> Self {
            Self(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
            )
        }
    }

    impl PluginEnvironment for FakeEnv {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key).filter(|v| !v.is_empty()).cloned()
        }
    }

    /// In-memory [`PluginProbe`] double: a set of installed absolute paths
    /// plus the version each installed binary advertises, keyed by file
    /// name.
    struct FakeProbe {
        installed: BTreeSet<PathBuf>,
        versions: BTreeMap<String, String>,
        silent: bool,
    }

    impl FakeProbe {
        /// Every listed plugin is installed under `dir` and advertises
        /// [`crate::version::CNI_SPEC_VERSION`].
        fn installed_under(dir: &str, plugins: &[&str]) -> Self {
            Self::installed_in(&[(dir, plugins)])
        }

        /// Install `plugins` under each `(dir, plugins)` pair.
        fn installed_in(dirs: &[(&str, &[&str])]) -> Self {
            let mut installed = BTreeSet::new();
            let mut versions = BTreeMap::new();
            for (dir, plugins) in dirs {
                for plugin in *plugins {
                    installed.insert(Path::new(dir).join(plugin));
                    versions.insert(
                        (*plugin).to_string(),
                        crate::version::CNI_SPEC_VERSION.to_string(),
                    );
                }
            }
            Self {
                installed,
                versions,
                silent: false,
            }
        }

        /// A probe where nothing is installed on any search path.
        fn empty() -> Self {
            Self {
                installed: BTreeSet::new(),
                versions: BTreeMap::new(),
                silent: false,
            }
        }

        /// Override what one installed binary claims to support.
        fn advertising(mut self, plugin: &str, version: &str) -> Self {
            self.versions
                .insert((*plugin).to_string(), (*version).to_string());
            self
        }

        /// A probe where every binary is installed but answers `VERSION`
        /// with an undecodable payload.
        fn silent(self) -> Self {
            Self {
                silent: true,
                ..self
            }
        }
    }

    impl PluginProbe for FakeProbe {
        fn is_installed(&self, candidate: &Path) -> bool {
            self.installed.contains(candidate)
        }

        fn advertised_spec_version(&self, binary: &Path) -> Result<String, CniError> {
            if self.silent {
                return Err(CniError::RolloutPreflight {
                    detail: format!("{} produced no decodable VERSION reply", binary.display()),
                });
            }
            let name = binary
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.versions
                .get(&name)
                .cloned()
                .ok_or_else(|| CniError::RolloutPreflight {
                    detail: format!("{} has no recorded VERSION reply", binary.display()),
                })
        }
    }

    fn conflist(version: &str, plugins: &[&str]) -> NetworkConfigList {
        NetworkConfigList {
            cni_version: version.to_string(),
            name: "minibox0".to_string(),
            plugins: plugins
                .iter()
                .map(|p| PluginConfig {
                    plugin_type: (*p).to_string(),
                    raw: serde_json::json!({"type": p}),
                })
                .collect(),
        }
    }

    fn rollout_with(cni_path: &str, config_dir: &str) -> CniRollout {
        CniRollout {
            network_mode: NETWORK_MODE.to_string(),
            cni_path: std::env::split_paths(cni_path).collect(),
            config_dir: PathBuf::from(config_dir),
            daemon_feature: DAEMON_FEATURE.to_string(),
            bridge_plugin: BRIDGE_PLUGIN_NAME.to_string(),
        }
    }

    #[test]
    fn defaults_match_the_daemon_environment() {
        let rollout = CniRollout::defaults();
        assert_eq!(rollout.cni_path, vec![PathBuf::from(DEFAULT_CNI_PATH)]);
        assert_eq!(rollout.config_dir, PathBuf::from(DEFAULT_CNI_CONFIG_DIR));
        assert_eq!(
            rollout.conflist_path().file_name().expect("filename"),
            CONFLIST_FILE_NAME
        );
        assert_eq!(rollout.daemon_feature, DAEMON_FEATURE);
        assert_eq!(rollout.network_mode, NETWORK_MODE);
    }

    #[test]
    fn conflist_filename_is_the_one_miniboxd_reads() {
        assert_eq!(CONFLIST_FILE_NAME, "10-minibox.conflist");
    }

    #[test]
    fn bridge_plugin_name_is_the_packaged_binary_name() {
        assert_eq!(BRIDGE_PLUGIN_NAME, "minibox-bridge");
    }

    #[test]
    fn from_environment_uses_defaults_when_nothing_is_set() {
        let rollout = CniRollout::from_environment(&FakeEnv::new(&[]));
        assert_eq!(rollout, CniRollout::defaults());
    }

    #[test]
    fn from_environment_applies_explicit_overrides() {
        let env = FakeEnv::new(&[
            (ENV_MINIBOX_NETWORK_MODE, "bridge"),
            (ENV_MINIBOX_CNI_PATH, "/srv/cni/bin"),
            (ENV_MINIBOX_CNI_CONFIG_DIR, "/srv/cni/net.d"),
        ]);
        let rollout = CniRollout::from_environment(&env);

        assert_eq!(rollout.cni_path, vec![PathBuf::from("/srv/cni/bin")]);
        assert_eq!(rollout.config_dir, PathBuf::from("/srv/cni/net.d"));
        assert_eq!(
            rollout.conflist_path(),
            PathBuf::from("/srv/cni/net.d/10-minibox.conflist")
        );
    }

    #[test]
    fn from_environment_splits_a_multi_entry_cni_path() {
        let env = FakeEnv::new(&[(ENV_MINIBOX_CNI_PATH, "/opt/cni/bin:/usr/libexec/cni")]);
        let rollout = CniRollout::from_environment(&env);
        assert_eq!(
            rollout.cni_path,
            vec![
                PathBuf::from("/opt/cni/bin"),
                PathBuf::from("/usr/libexec/cni")
            ]
        );
    }

    #[test]
    fn from_environment_treats_an_empty_override_as_unset() {
        let env = FakeEnv::new(&[(ENV_MINIBOX_CNI_PATH, ""), (ENV_MINIBOX_CNI_CONFIG_DIR, "")]);
        assert_eq!(CniRollout::from_environment(&env), CniRollout::defaults());
    }

    #[test]
    fn provider_arguments_are_what_miniboxd_constructs() {
        let rollout = rollout_with("/opt/cni/bin", "/etc/cni/net.d");
        let (cni_path, config_dir) = rollout.provider_arguments();
        assert_eq!(cni_path, vec![PathBuf::from("/opt/cni/bin")]);
        assert_eq!(config_dir, PathBuf::from("/etc/cni/net.d"));
    }

    #[test]
    fn install_hint_names_the_feature_binary_and_conflist() {
        let hint = CniRollout::defaults().install_hint();
        assert!(hint.contains(DAEMON_FEATURE), "{hint}");
        assert!(hint.contains(BRIDGE_PLUGIN_NAME), "{hint}");
        assert!(hint.contains(DEFAULT_CNI_PATH), "{hint}");
        assert!(
            hint.contains("/etc/cni/net.d/10-minibox.conflist"),
            "{hint}"
        );
        assert!(hint.contains(ENV_MINIBOX_NETWORK_MODE), "{hint}");
    }

    #[test]
    fn preflight_accepts_a_converged_rollout() {
        let rollout = rollout_with("/opt/cni/bin", "/etc/cni/net.d");
        let list = conflist(
            crate::version::CNI_SPEC_VERSION,
            &[BRIDGE_PLUGIN_NAME, "portmap"],
        );
        let probe = FakeProbe::installed_under("/opt/cni/bin", &[BRIDGE_PLUGIN_NAME, "portmap"]);

        let report = rollout
            .preflight(&list, &probe)
            .expect("converged rollout preflights cleanly");

        assert_eq!(report.network_name, "minibox0");
        assert_eq!(
            report.conflist_spec_version,
            crate::version::CNI_SPEC_VERSION
        );
        assert_eq!(report.plugins.len(), 2);
        assert_eq!(report.plugins[0].plugin_type, BRIDGE_PLUGIN_NAME);
        assert_eq!(
            report.plugins[0].path,
            PathBuf::from("/opt/cni/bin").join(BRIDGE_PLUGIN_NAME)
        );
        assert_eq!(
            report.plugins[0].advertised_spec_version,
            crate::version::CNI_SPEC_VERSION
        );
    }

    #[test]
    fn preflight_rejects_a_conflist_with_an_unsupported_version() {
        let rollout = rollout_with("/opt/cni/bin", "/etc/cni/net.d");
        let list = conflist("0.3.1", &[BRIDGE_PLUGIN_NAME]);
        let probe = FakeProbe::installed_under("/opt/cni/bin", &[BRIDGE_PLUGIN_NAME]);
        assert!(matches!(
            rollout.preflight(&list, &probe),
            Err(CniError::UnsupportedSpecVersion { .. })
        ));
    }

    #[test]
    fn preflight_rejects_a_plugin_advertising_an_unsupported_version() {
        let rollout = rollout_with("/opt/cni/bin", "/etc/cni/net.d");
        let list = conflist(crate::version::CNI_SPEC_VERSION, &[BRIDGE_PLUGIN_NAME]);
        let probe = FakeProbe::installed_under("/opt/cni/bin", &[BRIDGE_PLUGIN_NAME])
            .advertising(BRIDGE_PLUGIN_NAME, "0.3.1");
        assert!(matches!(
            rollout.preflight(&list, &probe),
            Err(CniError::UnsupportedSpecVersion { .. })
        ));
    }

    #[test]
    fn preflight_rejects_a_missing_plugin_binary() {
        let rollout = rollout_with("/opt/cni/bin", "/etc/cni/net.d");
        let list = conflist(crate::version::CNI_SPEC_VERSION, &[BRIDGE_PLUGIN_NAME]);
        let result = rollout.preflight(&list, &FakeProbe::empty());
        match result {
            Err(CniError::PluginNotFound { plugin, searched }) => {
                assert_eq!(plugin, BRIDGE_PLUGIN_NAME);
                assert_eq!(searched, vec![PathBuf::from("/opt/cni/bin")]);
            }
            other => panic!("expected PluginNotFound, got {other:?}"),
        }
    }

    #[test]
    fn preflight_rejects_a_plugin_that_will_not_answer_version() {
        let rollout = rollout_with("/opt/cni/bin", "/etc/cni/net.d");
        let list = conflist(crate::version::CNI_SPEC_VERSION, &[BRIDGE_PLUGIN_NAME]);
        let probe = FakeProbe::installed_under("/opt/cni/bin", &[BRIDGE_PLUGIN_NAME]).silent();
        let result = rollout.preflight(&list, &probe);
        match result {
            Err(CniError::RolloutPreflight { detail }) => {
                assert!(detail.contains(BRIDGE_PLUGIN_NAME), "{detail}");
            }
            other => panic!("expected RolloutPreflight, got {other:?}"),
        }
    }

    #[test]
    fn preflight_rejects_a_plugin_type_that_cannot_be_a_filename() {
        let rollout = rollout_with("/opt/cni/bin", "/etc/cni/net.d");
        let list = conflist(crate::version::CNI_SPEC_VERSION, &["../evil"]);
        let probe = FakeProbe::empty();
        assert!(matches!(
            rollout.preflight(&list, &probe),
            Err(CniError::InvalidPluginType { .. })
        ));
    }

    #[test]
    fn preflight_searches_cni_path_in_order() {
        let rollout = rollout_with("/first:/second", "/etc/cni/net.d");
        let list = conflist(crate::version::CNI_SPEC_VERSION, &[BRIDGE_PLUGIN_NAME]);
        // Only the second search directory has it installed.
        let probe = FakeProbe::installed_under("/second", &[BRIDGE_PLUGIN_NAME]);
        let report = rollout.preflight(&list, &probe).expect("preflight");
        assert_eq!(
            report.plugins[0].path,
            PathBuf::from("/second").join(BRIDGE_PLUGIN_NAME)
        );
    }

    #[test]
    fn preflight_prefers_the_earliest_search_directory() {
        let rollout = rollout_with("/first:/second", "/etc/cni/net.d");
        let list = conflist(crate::version::CNI_SPEC_VERSION, &[BRIDGE_PLUGIN_NAME]);
        let probe = FakeProbe::installed_in(&[
            ("/first", &[BRIDGE_PLUGIN_NAME]),
            ("/second", &[BRIDGE_PLUGIN_NAME]),
        ]);
        let report = rollout.preflight(&list, &probe).expect("preflight");
        assert_eq!(
            report.plugins[0].path,
            PathBuf::from("/first").join(BRIDGE_PLUGIN_NAME)
        );
    }

    #[test]
    fn default_rollout_matches_defaults() {
        assert_eq!(default_rollout(), CniRollout::defaults());
    }
}
