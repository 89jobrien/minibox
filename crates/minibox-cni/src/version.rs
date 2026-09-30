//! The single source of truth for the CNI spec version minibox speaks.
//!
//! Every spec-version string in this crate — the `VERSION` reply a plugin
//! returns, the validation the `ADD`/`DEL`/`CHECK` paths perform, and the
//! rollout preflight's check that installed plugins agree — is derived
//! from [`CNI_SPEC_VERSION`]. The version literal appears exactly once.

use crate::error::CniError;
use serde::{Deserialize, Serialize};

/// The CNI spec version minibox's plugins speak and accept.
///
/// This is the only place the literal is written down. Anything that
/// needs a spec-version string — a `VERSION` reply, a validation call,
/// an operator-facing preflight report — goes through
/// [`version_info`] or [`validate_spec_version`].
pub const CNI_SPEC_VERSION: &str = "1.0.0";

/// Spec versions the packaged plugins accept, in preference order.
///
/// Contains [`CNI_SPEC_VERSION`] as its first entry.
pub const SUPPORTED_SPEC_VERSIONS: &[&str] = &[CNI_SPEC_VERSION];

/// `CNI_COMMAND` value requesting interface creation.
pub const CNI_COMMAND_ADD: &str = "ADD";

/// `CNI_COMMAND` value requesting interface teardown.
pub const CNI_COMMAND_DEL: &str = "DEL";

/// `CNI_COMMAND` value requesting an existing attachment be verified.
pub const CNI_COMMAND_CHECK: &str = "CHECK";

/// `CNI_COMMAND` value requesting the supported spec versions.
pub const CNI_COMMAND_VERSION: &str = "VERSION";

/// Environment variable carrying the requested [`CniCommand`](crate::plugin::CniCommand).
pub const ENV_CNI_COMMAND: &str = "CNI_COMMAND";

/// Environment variable carrying the container identifier.
pub const ENV_CNI_CONTAINERID: &str = "CNI_CONTAINERID";

/// Environment variable carrying the network namespace target.
pub const ENV_CNI_NETNS: &str = "CNI_NETNS";

/// Environment variable carrying the interface name inside the namespace.
pub const ENV_CNI_IFNAME: &str = "CNI_IFNAME";

/// Environment variable carrying the plugin search path handed to nested plugins.
pub const ENV_CNI_PATH: &str = "CNI_PATH";

/// Environment variable carrying free-form `K=V;K2=V2` plugin arguments.
pub const ENV_CNI_ARGS: &str = "CNI_ARGS";

/// `CNI_COMMAND` values this crate recognises, in the order the spec lists them.
pub const CNI_COMMANDS: &[&str] = &[
    CNI_COMMAND_ADD,
    CNI_COMMAND_DEL,
    CNI_COMMAND_CHECK,
    CNI_COMMAND_VERSION,
];

/// The key holding the spec version inside a plugin config object.
pub const CONFIG_KEY_CNI_VERSION: &str = "cniVersion";

/// The key holding the network name inside a plugin config object.
pub const CONFIG_KEY_NAME: &str = "name";

/// CNI spec error code: the requested spec version is not supported.
pub const ERR_INCOMPATIBLE_CNI_VERSION: u32 = 1;

/// CNI spec error code: the config carries a field this plugin cannot honour.
pub const ERR_UNSUPPORTED_FIELD: u32 = 2;

/// CNI spec error code: the runtime did not supply the required environment.
pub const ERR_INVALID_ENVIRONMENT_VARIABLES: u32 = 4;

/// CNI spec error code: an I/O failure occurred.
pub const ERR_IO_FAILURE: u32 = 5;

/// CNI spec error code: the plugin config was not decodable.
pub const ERR_DECODING_FAILURE: u32 = 6;

/// CNI spec error code: the network config was rejected.
pub const ERR_INVALID_NETWORK_CONFIG: u32 = 7;

/// CNI spec error code: the network namespace target was unusable.
pub const ERR_INVALID_NETNS: u32 = 8;

/// CNI spec error code: the operation is retryable.
pub const ERR_TRY_AGAIN_LATER: u32 = 11;

/// CNI spec error code: the plugin hit an internal failure.
pub const ERR_INTERNAL: u32 = 999;

/// Reply to `CNI_COMMAND=VERSION`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VersionInfo {
    /// The spec version this plugin was built against.
    #[serde(rename = "cniVersion")]
    pub cni_version: String,
    /// Every spec version this plugin accepts, in preference order.
    #[serde(rename = "supportedVersions")]
    pub supported_versions: Vec<String>,
}

/// Build the `VERSION` reply from the crate's constants.
///
/// # Examples
///
/// ```
/// let info = minibox_cni::version_info();
/// assert_eq!(info.cni_version, minibox_cni::CNI_SPEC_VERSION);
/// assert!(info.supported_versions.contains(&minibox_cni::CNI_SPEC_VERSION.to_string()));
/// ```
#[must_use]
pub fn version_info() -> VersionInfo {
    VersionInfo {
        cni_version: CNI_SPEC_VERSION.to_string(),
        supported_versions: SUPPORTED_SPEC_VERSIONS
            .iter()
            .map(|version| (*version).to_string())
            .collect(),
    }
}

/// Check a requested spec version against [`SUPPORTED_SPEC_VERSIONS`].
///
/// On success returns the matched supported version. This is the only
/// acceptance gate for spec versions: the `VERSION` reply, the
/// `ADD`/`DEL`/`CHECK` request validation, and the rollout preflight all
/// route through it, so a version can never be accepted in one path and
/// rejected in another.
///
/// # Errors
///
/// Returns [`CniError::UnsupportedSpecVersion`] listing the supported
/// versions when `requested` matches none of them.
pub fn validate_spec_version(requested: &str) -> Result<&'static str, CniError> {
    SUPPORTED_SPEC_VERSIONS
        .iter()
        .copied()
        .find(|supported| *supported == requested)
        .ok_or_else(|| CniError::UnsupportedSpecVersion {
            requested: requested.to_string(),
            supported: SUPPORTED_SPEC_VERSIONS
                .iter()
                .map(|version| (*version).to_string())
                .collect(),
        })
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    const EXPECTED_SPEC_VERSION: &str = "1.0.0";

    #[test]
    fn spec_version_constant_is_the_expected_cni_spec_version() {
        assert_eq!(CNI_SPEC_VERSION, EXPECTED_SPEC_VERSION);
    }

    #[test]
    fn supported_versions_contains_the_spec_version_constant() {
        assert!(
            SUPPORTED_SPEC_VERSIONS.contains(&CNI_SPEC_VERSION),
            "SUPPORTED_SPEC_VERSIONS must lead with CNI_SPEC_VERSION, got {SUPPORTED_SPEC_VERSIONS:?}"
        );
    }

    #[test]
    fn version_info_cni_version_agrees_with_the_spec_version_constant() {
        assert_eq!(version_info().cni_version, CNI_SPEC_VERSION);
    }

    #[test]
    fn version_info_reports_exactly_the_supported_versions() {
        let info = version_info();
        let expected: Vec<String> = SUPPORTED_SPEC_VERSIONS
            .iter()
            .map(|v| (*v).to_string())
            .collect();
        assert_eq!(info.supported_versions, expected);
    }

    #[test]
    fn version_info_serializes_to_the_cni_version_reply_shape() {
        let value = serde_json::to_value(version_info()).expect("serialize");
        assert_eq!(value["cniVersion"], EXPECTED_SPEC_VERSION);
        let supported = value["supportedVersions"]
            .as_array()
            .expect("supportedVersions must be an array");
        assert!(
            supported.iter().any(|v| v == EXPECTED_SPEC_VERSION),
            "supportedVersions {supported:?} must advertise {EXPECTED_SPEC_VERSION}"
        );
    }

    #[test]
    fn validate_spec_version_accepts_the_supported_version() {
        for supported in SUPPORTED_SPEC_VERSIONS {
            assert_eq!(
                validate_spec_version(supported).expect("supported version accepted"),
                *supported
            );
        }
    }

    #[test]
    fn validate_spec_version_rejects_an_unsupported_version() {
        let result = validate_spec_version("0.3.1");
        match result {
            Err(CniError::UnsupportedSpecVersion {
                requested,
                supported,
            }) => {
                assert_eq!(requested, "0.3.1");
                assert_eq!(supported, SUPPORTED_SPEC_VERSIONS);
            }
            other => panic!("expected UnsupportedSpecVersion, got {other:?}"),
        }
    }

    #[test]
    fn validate_spec_version_rejects_a_malformed_version() {
        for bogus in ["", "banana", "1", "1.0", "v1.0.0"] {
            assert!(
                validate_spec_version(bogus).is_err(),
                "{bogus:?} must not be accepted as a spec version"
            );
        }
    }

    #[test]
    fn cni_commands_cover_the_full_lifecycle() {
        for command in ["ADD", "DEL", "CHECK", "VERSION"] {
            assert!(
                CNI_COMMANDS.contains(&command),
                "CNI_COMMANDS must include {command}"
            );
        }
        assert_eq!(CNI_COMMANDS.len(), 4);
    }
}
