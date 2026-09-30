//! Plugin-side CNI exec protocol: decoding an invocation and dispatching
//! a [`CniCommand`] to a [`NamespaceAttachment`] backend.
//!
//! Everything in this module is a pure function of its inputs. The two
//! effects an invocation has — reading the process environment, and
//! mutating a network namespace — are both expressed as ports
//! ([`PluginEnvironment`] and [`NamespaceAttachment`]) so the protocol
//! logic can be exercised in-memory, with no root, no network namespace,
//! and no CNI plugin on disk.

use crate::error::CniError;
use crate::result::CniResult;
use crate::version::{
    self, CNI_COMMAND_ADD, CNI_COMMAND_CHECK, CNI_COMMAND_DEL, CNI_COMMAND_VERSION,
    CONFIG_KEY_CNI_VERSION, CONFIG_KEY_NAME, ENV_CNI_ARGS, ENV_CNI_COMMAND, ENV_CNI_CONTAINERID,
    ENV_CNI_IFNAME, ENV_CNI_NETNS, ENV_CNI_PATH, ERR_DECODING_FAILURE,
    ERR_INCOMPATIBLE_CNI_VERSION, ERR_INTERNAL, ERR_INVALID_ENVIRONMENT_VARIABLES,
    ERR_INVALID_NETNS, ERR_INVALID_NETWORK_CONFIG, ERR_IO_FAILURE, validate_spec_version,
    version_info,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::PathBuf;

/// The config key naming the host bridge this plugin attaches to.
pub const CONFIG_KEY_BRIDGE: &str = "bridge";

/// A `CNI_COMMAND` value this plugin implements.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CniCommand {
    /// Create the namespace-scoped interface.
    Add,
    /// Remove the namespace-scoped interface.
    Del,
    /// Verify the namespace-scoped interface.
    Check,
    /// Report the supported spec versions.
    Version,
}

impl CniCommand {
    /// The `CNI_COMMAND` literal this variant is decoded from.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Add => CNI_COMMAND_ADD,
            Self::Del => CNI_COMMAND_DEL,
            Self::Check => CNI_COMMAND_CHECK,
            Self::Version => CNI_COMMAND_VERSION,
        }
    }

    /// Whether this command performs network-namespace work and therefore
    /// requires the container, namespace, and interface environment.
    #[must_use]
    pub const fn requires_namespace(self) -> bool {
        !matches!(self, Self::Version)
    }
}

/// Decode a `CNI_COMMAND` value.
///
/// Matching is exact and case-sensitive, per the CNI spec.
///
/// # Errors
///
/// Returns [`CniError::UnsupportedCommand`] for any value outside
/// `ADD`/`DEL`/`CHECK`/`VERSION`.
pub fn command_from_str(raw: &str) -> Result<CniCommand, CniError> {
    match raw {
        CNI_COMMAND_ADD => Ok(CniCommand::Add),
        CNI_COMMAND_DEL => Ok(CniCommand::Del),
        CNI_COMMAND_CHECK => Ok(CniCommand::Check),
        CNI_COMMAND_VERSION => Ok(CniCommand::Version),
        other => Err(CniError::UnsupportedCommand {
            command: other.to_string(),
        }),
    }
}

/// Port supplying the process environment an invocation is decoded from.
///
/// Implemented by [`crate::adapters::env::ProcessEnvironment`] in
/// production and by an in-memory map in tests.
pub trait PluginEnvironment {
    /// Return the value of `key`, or `None` when it is unset or empty.
    fn get(&self, key: &str) -> Option<String>;
}

/// The namespace-scoped facts an [`NamespaceAttachment`] backend needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentRequest {
    /// Network name from the plugin config.
    pub network_name: String,
    /// `CNI_CONTAINERID`.
    pub container_id: String,
    /// `CNI_IFNAME`.
    pub ifname: String,
    /// `CNI_NETNS`, passed through opaquely.
    pub netns: String,
    /// Host bridge named by the config's `bridge` field, if any.
    pub bridge: Option<String>,
    /// `CNI_PATH`, so a backend can chain into further plugins.
    pub cni_path: Vec<PathBuf>,
    /// Parsed `CNI_ARGS`.
    pub cni_args: BTreeMap<String, Option<String>>,
}

impl AttachmentRequest {
    /// The result an `ADD` or `CHECK` of this request reports back, before
    /// any backend has touched the namespace.
    #[must_use]
    pub fn empty_result(&self) -> CniResult {
        CniResult {
            cni_version: version::CNI_SPEC_VERSION.to_string(),
            interfaces: Vec::new(),
            ips: Vec::new(),
            dns: crate::result::CniDns::default(),
        }
    }
}

/// Port performing the network-namespace work behind `ADD`/`DEL`/`CHECK`.
///
/// The packaged plugin ships an adapter that reports a structured CNI
/// error rather than guessing; a real netlink implementation is the
/// remaining piece. Tests substitute an in-memory recorder.
pub trait NamespaceAttachment: Send + Sync {
    /// Create the namespace-scoped interface described by `request`.
    ///
    /// # Errors
    ///
    /// Returns a [`CniError`] the plugin reports as a structured CNI error.
    fn attach(&self, request: &AttachmentRequest) -> Result<CniResult, CniError>;

    /// Verify the interface described by `request` is present and correct.
    ///
    /// # Errors
    ///
    /// Returns a [`CniError`] the plugin reports as a structured CNI error.
    fn check(&self, request: &AttachmentRequest) -> Result<CniResult, CniError>;

    /// Remove the namespace-scoped interface described by `request`.
    ///
    /// Implementations are expected to be idempotent; the plugin's `DEL`
    /// path already tolerates a backend that reports teardown of a
    /// resource it cannot find.
    ///
    /// # Errors
    ///
    /// Returns a [`CniError`] the plugin reports as a structured CNI error.
    fn detach(&self, request: &AttachmentRequest) -> Result<(), CniError>;
}

/// A decoded CNI plugin invocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginRequest {
    /// The requested command.
    pub command: CniCommand,
    /// The spec version the caller asked for, already validated against
    /// [`version::SUPPORTED_SPEC_VERSIONS`].
    pub spec_version: String,
    /// Network name from the plugin config.
    pub network_name: String,
    /// Host bridge named by the config's `bridge` field, if any.
    pub bridge: Option<String>,
    /// The plugin config exactly as received on stdin.
    pub raw_config: serde_json::Value,
    /// The namespace-scoped facts, empty for [`CniCommand::Version`].
    pub attachment: AttachmentRequest,
}

impl PluginRequest {
    /// Decode an invocation from `env` and the `stdin` config bytes.
    ///
    /// The spec version in the config is validated against
    /// [`version::SUPPORTED_SPEC_VERSIONS`], so an unsupported version is
    /// rejected before any backend is consulted. `CNI_COMMAND=VERSION` is
    /// exempt from both the config and the namespace environment, per the
    /// spec.
    ///
    /// # Errors
    ///
    /// Returns [`CniError::MissingEnvVar`] when a required `CNI_*` variable
    /// is unset, [`CniError::UnsupportedCommand`] for an unknown command,
    /// [`CniError::ConfigParse`] when stdin is not valid JSON, and
    /// [`CniError::UnsupportedSpecVersion`] or
    /// [`CniError::MissingConfigField`] when the config's `cniVersion` or
    /// `name` is missing or unusable.
    pub fn decode(env: &dyn PluginEnvironment, stdin: &[u8]) -> Result<Self, CniError> {
        let command = command_from_str(&required(env, ENV_CNI_COMMAND)?)?;
        if command == CniCommand::Version {
            return Ok(Self {
                command,
                spec_version: version::CNI_SPEC_VERSION.to_string(),
                network_name: String::new(),
                bridge: None,
                raw_config: serde_json::Value::Null,
                attachment: AttachmentRequest {
                    network_name: String::new(),
                    container_id: String::new(),
                    ifname: String::new(),
                    netns: String::new(),
                    bridge: None,
                    cni_path: Vec::new(),
                    cni_args: BTreeMap::new(),
                },
            });
        }
        let config = decode_config(stdin)?;
        let requested = config
            .get(CONFIG_KEY_CNI_VERSION)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| CniError::MissingConfigField {
                field: CONFIG_KEY_CNI_VERSION.to_string(),
            })?;
        let spec_version = validate_spec_version(requested)?.to_string();
        let network_name = config
            .get(CONFIG_KEY_NAME)
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| CniError::MissingConfigField {
                field: CONFIG_KEY_NAME.to_string(),
            })?
            .to_string();
        let bridge = config
            .get(CONFIG_KEY_BRIDGE)
            .and_then(serde_json::Value::as_str)
            .map(ToString::to_string);
        let cni_path = env
            .get(ENV_CNI_PATH)
            .map(|raw| std::env::split_paths(&raw).collect())
            .unwrap_or_default();
        let cni_args = env
            .get(ENV_CNI_ARGS)
            .map(|raw| parse_cni_args(&raw))
            .unwrap_or_default();
        let container_id = required(env, ENV_CNI_CONTAINERID)?;
        let ifname = required(env, ENV_CNI_IFNAME)?;
        Ok(Self {
            command,
            spec_version,
            network_name: network_name.clone(),
            bridge: bridge.clone(),
            raw_config: config,
            attachment: AttachmentRequest {
                network_name,
                container_id,
                ifname,
                netns: required(env, ENV_CNI_NETNS)?,
                bridge,
                cni_path,
                cni_args,
            },
        })
    }

    /// The CNI result this request's `ADD`/`CHECK` reports when no backend
    /// work has been performed.
    #[must_use]
    pub fn empty_result(&self) -> CniResult {
        self.attachment.empty_result()
    }
}

fn decode_config(stdin: &[u8]) -> Result<serde_json::Value, CniError> {
    if stdin.iter().all(u8::is_ascii_whitespace) {
        return Err(CniError::MissingConfigField {
            field: CONFIG_KEY_CNI_VERSION.to_string(),
        });
    }
    // Decoding straight into a Map rejects arrays and scalars with a
    // decode failure, which is exactly the spec's "config is not an
    // object" case.
    let map: serde_json::Map<String, serde_json::Value> = serde_json::from_slice(stdin)?;
    Ok(serde_json::Value::Object(map))
}

fn required(env: &dyn PluginEnvironment, key: &str) -> Result<String, CniError> {
    env.get(key).ok_or_else(|| CniError::MissingEnvVar {
        var: key.to_string(),
    })
}

/// Parse the spec's `CNI_ARGS` `K=V;K2=V2` form.
///
/// Segments without `=` become keys with no value, and empty segments are
/// skipped, so a trailing `;` is harmless.
#[must_use]
pub fn parse_cni_args(raw: &str) -> BTreeMap<String, Option<String>> {
    raw.split(';')
        .map(str::trim)
        .filter(|segment| !segment.is_empty())
        .map(|segment| match segment.split_once('=') {
            Some((key, value)) => (key.to_string(), Some(value.to_string())),
            None => (segment.to_string(), None),
        })
        .collect()
}

/// Run a decoded request against `backend`, returning the JSON to print on
/// stdout.
///
/// `VERSION` short-circuits to the crate's [`version_info`]; `ADD` and
/// `CHECK` return the backend's result verbatim; `DEL` returns JSON `null`
/// because the spec gives teardown no result object.
///
/// # Errors
///
/// Returns whatever the backend reports, or
/// [`CniError::ConfigParse`] if a result cannot be serialised.
pub fn dispatch(
    request: &PluginRequest,
    backend: &dyn NamespaceAttachment,
) -> Result<serde_json::Value, CniError> {
    match request.command {
        CniCommand::Version => Ok(serde_json::to_value(version_info())?),
        CniCommand::Add => Ok(serde_json::to_value(backend.attach(&request.attachment)?)?),
        CniCommand::Check => Ok(serde_json::to_value(backend.check(&request.attachment)?)?),
        CniCommand::Del => {
            backend.detach(&request.attachment)?;
            Ok(serde_json::Value::Null)
        }
    }
}

/// Map a crate error onto the spec's structured error payload.
///
/// Every [`CniError`] maps to a defined CNI spec code, so a plugin built
/// on this crate never exits non-zero with an undecodable stdout.
///
/// [`CniError::PluginError`] passes the code and message a downstream
/// plugin reported straight through, which is what a runtime chaining
/// minibox plugins expects to see.
#[must_use]
pub fn error_payload(error: &CniError) -> crate::result::CniErrorPayload {
    match error {
        // A downstream plugin's own code and message are more specific than
        // anything this crate could synthesise, so they pass straight
        // through — that is what a runtime chaining minibox plugins expects.
        CniError::PluginError {
            code, msg, details, ..
        } => crate::result::CniErrorPayload {
            code: code.unwrap_or(ERR_INTERNAL),
            msg: msg.clone(),
            details: details.clone(),
        },
        CniError::UnsupportedSpecVersion { .. } => payload(ERR_INCOMPATIBLE_CNI_VERSION, error),
        CniError::MissingEnvVar { var } if var == ENV_CNI_NETNS => {
            payload(ERR_INVALID_NETNS, error)
        }
        CniError::MissingEnvVar { .. } | CniError::UnsupportedCommand { .. } => {
            payload(ERR_INVALID_ENVIRONMENT_VARIABLES, error)
        }
        CniError::ConfigParse(_) => payload(ERR_DECODING_FAILURE, error),
        CniError::Io(_) => payload(ERR_IO_FAILURE, error),
        CniError::MissingConfigField { .. }
        | CniError::RolloutPreflight { .. }
        | CniError::InvalidPluginType { .. }
        | CniError::PluginNotFound { .. } => payload(ERR_INVALID_NETWORK_CONFIG, error),
        CniError::ProcessFailed { .. } => payload(ERR_INTERNAL, error),
    }
}

fn payload(code: u32, error: &CniError) -> crate::result::CniErrorPayload {
    crate::result::CniErrorPayload {
        code,
        msg: error.to_string(),
        details: None,
    }
}

/// The spec code [`error_payload`] would assign to `error`.
#[must_use]
pub fn error_code(error: &CniError) -> u32 {
    error_payload(error).code
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    const BRIDGE_CONFIG: &str =
        r#"{"cniVersion":"1.0.0","name":"minibox0","type":"minibox-bridge","bridge":"minibox0"}"#;

    /// In-memory [`PluginEnvironment`] double.
    struct FakeEnvironment(BTreeMap<String, String>);

    impl FakeEnvironment {
        fn new(pairs: &[(&str, &str)]) -> Self {
            Self(
                pairs
                    .iter()
                    .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
                    .collect(),
            )
        }

        fn add_namespace_env(self) -> Self {
            Self(BTreeMap::from_iter([
                (ENV_CNI_CONTAINERID.to_string(), "container-1".to_string()),
                (ENV_CNI_NETNS.to_string(), "/proc/1/ns/net".to_string()),
                (ENV_CNI_IFNAME.to_string(), "eth0".to_string()),
            ]))
            .merged(self)
        }

        fn merged(mut self, other: Self) -> Self {
            self.0.extend(other.0);
            self
        }
    }

    impl PluginEnvironment for FakeEnvironment {
        fn get(&self, key: &str) -> Option<String> {
            self.0.get(key).filter(|v| !v.is_empty()).cloned()
        }
    }

    /// In-memory [`NamespaceAttachment`] double recording what it was asked.
    #[derive(Default)]
    struct RecordingAttachment {
        calls: Mutex<Vec<&'static str>>,
        seen: Mutex<Vec<AttachmentRequest>>,
        fail_attach: bool,
    }

    impl RecordingAttachment {
        fn failing_attach() -> Self {
            Self {
                fail_attach: true,
                ..Self::default()
            }
        }
    }

    impl NamespaceAttachment for RecordingAttachment {
        fn attach(&self, request: &AttachmentRequest) -> Result<CniResult, CniError> {
            self.calls.lock().expect("calls lock").push("attach");
            self.seen.lock().expect("seen lock").push(request.clone());
            if self.fail_attach {
                return Err(CniError::PluginError {
                    plugin: "minibox-bridge".to_string(),
                    code: Some(7),
                    msg: "no free addresses".to_string(),
                    details: None,
                });
            }
            Ok(CniResult {
                cni_version: version::CNI_SPEC_VERSION.to_string(),
                interfaces: vec![crate::result::CniInterface {
                    name: request.ifname.clone(),
                    mac: None,
                    sandbox: Some(request.netns.clone()),
                }],
                ips: Vec::new(),
                dns: crate::result::CniDns::default(),
            })
        }

        fn check(&self, request: &AttachmentRequest) -> Result<CniResult, CniError> {
            self.calls.lock().expect("calls lock").push("check");
            Ok(request.empty_result())
        }

        fn detach(&self, request: &AttachmentRequest) -> Result<(), CniError> {
            self.calls.lock().expect("calls lock").push("detach");
            self.seen.lock().expect("seen lock").push(request.clone());
            Ok(())
        }
    }

    fn add_env() -> FakeEnvironment {
        FakeEnvironment::new(&[(ENV_CNI_COMMAND, "ADD")]).add_namespace_env()
    }

    #[test]
    fn command_from_str_maps_every_supported_command() {
        assert_eq!(command_from_str("ADD").expect("ADD"), CniCommand::Add);
        assert_eq!(command_from_str("DEL").expect("DEL"), CniCommand::Del);
        assert_eq!(command_from_str("CHECK").expect("CHECK"), CniCommand::Check);
        assert_eq!(
            command_from_str("VERSION").expect("VERSION"),
            CniCommand::Version
        );
    }

    #[test]
    fn command_as_str_round_trips_every_command() {
        for raw in version::CNI_COMMANDS {
            let command = command_from_str(raw).expect("supported command");
            assert_eq!(command.as_str(), *raw);
        }
    }

    #[test]
    fn command_from_str_rejects_an_unknown_command() {
        for bogus in ["", "add", "Add", "GC", "ADD;rm -rf /"] {
            match command_from_str(bogus) {
                Err(CniError::UnsupportedCommand { command }) => assert_eq!(command, bogus),
                other => panic!("expected UnsupportedCommand for {bogus:?}, got {other:?}"),
            }
        }
    }

    #[test]
    fn only_version_skips_the_namespace_environment() {
        assert!(!CniCommand::Version.requires_namespace());
        for command in [CniCommand::Add, CniCommand::Del, CniCommand::Check] {
            assert!(command.requires_namespace(), "{command:?} needs a netns");
        }
    }

    #[test]
    fn decode_accepts_the_supported_spec_version() {
        let request = PluginRequest::decode(&add_env(), BRIDGE_CONFIG.as_bytes()).expect("decode");
        assert_eq!(request.command, CniCommand::Add);
        assert_eq!(request.spec_version, version::CNI_SPEC_VERSION);
        assert_eq!(request.network_name, "minibox0");
        assert_eq!(request.bridge.as_deref(), Some("minibox0"));
        assert_eq!(request.attachment.container_id, "container-1");
        assert_eq!(request.attachment.ifname, "eth0");
        assert_eq!(request.attachment.netns, "/proc/1/ns/net");
    }

    #[test]
    fn decode_rejects_an_unsupported_spec_version() {
        let env = add_env();
        let config = r#"{"cniVersion":"0.3.1","name":"minibox0","type":"minibox-bridge"}"#;
        let result = PluginRequest::decode(&env, config.as_bytes());
        assert!(matches!(
            result,
            Err(CniError::UnsupportedSpecVersion { .. })
        ));
    }

    #[test]
    fn decode_rejects_a_missing_spec_version() {
        let env = add_env();
        let result = PluginRequest::decode(&env, br#"{"name":"minibox0"}"#);
        match result {
            Err(CniError::MissingConfigField { field }) => assert_eq!(field, "cniVersion"),
            other => panic!("expected MissingConfigField, got {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_a_missing_network_name() {
        let env = add_env();
        let result = PluginRequest::decode(&env, br#"{"cniVersion":"1.0.0"}"#);
        match result {
            Err(CniError::MissingConfigField { field }) => assert_eq!(field, "name"),
            other => panic!("expected MissingConfigField, got {other:?}"),
        }
    }

    #[test]
    fn decode_rejects_a_missing_cni_command() {
        let result = PluginRequest::decode(&FakeEnvironment::new(&[]), b"");
        match result {
            Err(CniError::MissingEnvVar { var }) => assert_eq!(var, ENV_CNI_COMMAND),
            other => panic!("expected MissingEnvVar, got {other:?}"),
        }
    }

    #[test]
    fn decode_requires_the_namespace_environment_for_add() {
        for missing in [ENV_CNI_CONTAINERID, ENV_CNI_NETNS, ENV_CNI_IFNAME] {
            let pairs: Vec<(&str, &str)> = [
                (ENV_CNI_COMMAND, "ADD"),
                (ENV_CNI_CONTAINERID, "container-1"),
                (ENV_CNI_NETNS, "/proc/1/ns/net"),
                (ENV_CNI_IFNAME, "eth0"),
            ]
            .into_iter()
            .filter(|(k, _)| *k != missing)
            .collect();
            let result =
                PluginRequest::decode(&FakeEnvironment::new(&pairs), BRIDGE_CONFIG.as_bytes());
            match result {
                Err(CniError::MissingEnvVar { var }) => assert_eq!(var, missing),
                other => panic!("expected MissingEnvVar({missing}), got {other:?}"),
            }
        }
    }

    #[test]
    fn decode_of_version_needs_neither_config_nor_namespace() {
        let env = FakeEnvironment::new(&[(ENV_CNI_COMMAND, "VERSION")]);
        let request = PluginRequest::decode(&env, b"").expect("VERSION needs nothing else");
        assert_eq!(request.command, CniCommand::Version);
        assert_eq!(request.spec_version, version::CNI_SPEC_VERSION);
        assert!(request.raw_config.is_null());
    }

    #[test]
    fn decode_rejects_a_non_object_config() {
        let env = add_env();
        let result = PluginRequest::decode(&env, b"[1, 2, 3]");
        assert!(matches!(result, Err(CniError::ConfigParse(_))));
    }

    #[test]
    fn decode_defaults_optional_cni_path_and_args() {
        let request = PluginRequest::decode(&add_env(), BRIDGE_CONFIG.as_bytes()).expect("decode");
        assert!(request.attachment.cni_path.is_empty());
        assert!(request.attachment.cni_args.is_empty());
    }

    #[test]
    fn decode_splits_cni_path_and_parses_cni_args() {
        let env = add_env().merged(FakeEnvironment::new(&[
            (ENV_CNI_PATH, "/opt/cni/bin:/usr/libexec/cni"),
            (ENV_CNI_ARGS, "K8S_POD_NAMESPACE=default;IP=;DEBUG"),
        ]));
        let request = PluginRequest::decode(&env, BRIDGE_CONFIG.as_bytes()).expect("decode");
        assert_eq!(
            request.attachment.cni_path,
            vec![
                PathBuf::from("/opt/cni/bin"),
                PathBuf::from("/usr/libexec/cni")
            ]
        );
        assert_eq!(
            request.attachment.cni_args.get("K8S_POD_NAMESPACE"),
            Some(&Some("default".to_string()))
        );
        assert_eq!(
            request.attachment.cni_args.get("IP"),
            Some(&Some(String::new()))
        );
        assert_eq!(request.attachment.cni_args.get("DEBUG"), Some(&None));
    }

    #[test]
    fn parse_cni_args_handles_values_bare_keys_and_empty_segments() {
        let args = parse_cni_args("A=1;B;C=2=3;;  ;D=");
        assert_eq!(args.get("A"), Some(&Some("1".to_string())));
        assert_eq!(args.get("B"), Some(&None));
        assert_eq!(args.get("C"), Some(&Some("2=3".to_string())));
        assert_eq!(args.get("D"), Some(&Some(String::new())));
        assert!(parse_cni_args("").is_empty());
        assert!(parse_cni_args(";;;").is_empty());
    }

    #[test]
    fn empty_result_reports_the_spec_version_constant() {
        let request = PluginRequest::decode(&add_env(), BRIDGE_CONFIG.as_bytes()).expect("decode");
        assert_eq!(
            request.empty_result().cni_version,
            version::CNI_SPEC_VERSION
        );
    }

    #[test]
    fn dispatch_version_returns_the_single_source_of_truth() {
        let env = FakeEnvironment::new(&[(ENV_CNI_COMMAND, "VERSION")]);
        let request = PluginRequest::decode(&env, b"").expect("decode");
        let value = dispatch(&request, &RecordingAttachment::default()).expect("dispatch");

        assert_eq!(value["cniVersion"], version::CNI_SPEC_VERSION);
        let supported = value["supportedVersions"]
            .as_array()
            .expect("supportedVersions array");
        assert!(
            supported.iter().any(|v| v == version::CNI_SPEC_VERSION),
            "VERSION reply {value} must advertise the crate constant"
        );
    }

    #[test]
    fn dispatch_add_delegates_to_the_attachment_port() {
        let backend = RecordingAttachment::default();
        let request = PluginRequest::decode(&add_env(), BRIDGE_CONFIG.as_bytes()).expect("decode");
        let value = dispatch(&request, &backend).expect("dispatch");

        assert_eq!(backend.calls.lock().expect("calls").as_slice(), ["attach"]);
        assert_eq!(value["cniVersion"], version::CNI_SPEC_VERSION);
        assert_eq!(value["interfaces"][0]["name"], "eth0");
    }

    #[test]
    fn dispatch_check_delegates_to_the_attachment_port() {
        let env = FakeEnvironment::new(&[(ENV_CNI_COMMAND, "CHECK")]).add_namespace_env();
        let backend = RecordingAttachment::default();
        let request = PluginRequest::decode(&env, BRIDGE_CONFIG.as_bytes()).expect("decode");
        let value = dispatch(&request, &backend).expect("dispatch");

        assert_eq!(backend.calls.lock().expect("calls").as_slice(), ["check"]);
        assert_eq!(value["cniVersion"], version::CNI_SPEC_VERSION);
    }

    #[test]
    fn dispatch_del_is_idempotent_and_returns_null() {
        let env = FakeEnvironment::new(&[(ENV_CNI_COMMAND, "DEL")]).add_namespace_env();
        let backend = RecordingAttachment::default();
        let request = PluginRequest::decode(&env, BRIDGE_CONFIG.as_bytes()).expect("decode");
        let value = dispatch(&request, &backend).expect("dispatch");

        assert_eq!(backend.calls.lock().expect("calls").as_slice(), ["detach"]);
        assert!(value.is_null(), "DEL must print null, got {value}");
    }

    #[test]
    fn dispatch_propagates_a_backend_failure() {
        let backend = RecordingAttachment::failing_attach();
        let request = PluginRequest::decode(&add_env(), BRIDGE_CONFIG.as_bytes()).expect("decode");
        let result = dispatch(&request, &backend);
        assert!(matches!(
            result,
            Err(CniError::PluginError { code: Some(7), .. })
        ));
    }

    #[test]
    fn error_payload_passes_through_a_downstream_plugin_code() {
        let error = CniError::PluginError {
            plugin: "portmap".to_string(),
            code: Some(7),
            msg: "no free addresses".to_string(),
            details: Some("pool exhausted".to_string()),
        };
        let payload = error_payload(&error);
        assert_eq!(payload.code, 7);
        assert_eq!(payload.msg, "no free addresses");
        assert_eq!(payload.details.as_deref(), Some("pool exhausted"));
    }

    #[test]
    fn error_payload_uses_the_spec_code_for_each_crate_error() {
        let cases: Vec<(CniError, u32)> = vec![
            (
                CniError::UnsupportedSpecVersion {
                    requested: "0.3.1".to_string(),
                    supported: vec![version::CNI_SPEC_VERSION.to_string()],
                },
                ERR_INCOMPATIBLE_CNI_VERSION,
            ),
            (
                CniError::MissingEnvVar {
                    var: ENV_CNI_CONTAINERID.to_string(),
                },
                ERR_INVALID_ENVIRONMENT_VARIABLES,
            ),
            (
                CniError::MissingEnvVar {
                    var: ENV_CNI_NETNS.to_string(),
                },
                ERR_INVALID_NETNS,
            ),
            (
                CniError::UnsupportedCommand {
                    command: "GC".to_string(),
                },
                ERR_INVALID_ENVIRONMENT_VARIABLES,
            ),
            (
                CniError::MissingConfigField {
                    field: "cniVersion".to_string(),
                },
                ERR_INVALID_NETWORK_CONFIG,
            ),
            (
                CniError::RolloutPreflight {
                    detail: "missing".to_string(),
                },
                ERR_INVALID_NETWORK_CONFIG,
            ),
            (
                CniError::InvalidPluginType {
                    plugin: "../evil".to_string(),
                },
                ERR_INVALID_NETWORK_CONFIG,
            ),
            (
                CniError::PluginNotFound {
                    plugin: "bridge".to_string(),
                    searched: vec![PathBuf::from("/opt/cni/bin")],
                },
                ERR_INVALID_NETWORK_CONFIG,
            ),
            (
                CniError::ProcessFailed {
                    plugin: "bridge".to_string(),
                    exit_code: Some(2),
                    stderr: String::new(),
                },
                ERR_INTERNAL,
            ),
            (CniError::Io(std::io::Error::other("boom")), ERR_IO_FAILURE),
        ];
        for (error, expected) in cases {
            assert_eq!(
                error_code(&error),
                expected,
                "{error} should map to spec code {expected}"
            );
        }
    }

    #[test]
    fn error_payload_for_a_decode_failure_is_decoding_failure() {
        let error = CniError::ConfigParse(
            serde_json::from_str::<serde_json::Value>("{").expect_err("malformed json"),
        );
        assert_eq!(error_code(&error), ERR_DECODING_FAILURE);
    }

    #[test]
    fn error_payload_always_carries_a_message() {
        let error = CniError::MissingEnvVar {
            var: ENV_CNI_IFNAME.to_string(),
        };
        let payload = error_payload(&error);
        assert!(!payload.msg.is_empty());
        assert!(payload.msg.contains(ENV_CNI_IFNAME));
    }
}
