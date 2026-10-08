//! Versioned isolation profile contract and deterministic digests.
//!
//! This module defines the portable, deterministic isolation vocabulary
//! consumed by admission, runtimes, protocols, manifests, and conformance
//! tests. It implements
//! `docs/designs/2026-09-21-isolation-profile-contract-design.md`.
//!
//! Only two built-in profiles exist today: [`standard_v1`] (the global
//! default) and [`compatibility_v1`] (an explicit, operator-authorized escape
//! hatch). Caller-authored profiles are out of scope for this contract.
//! Runtime enforcement, wire protocol, and adapter qualification are separate
//! concerns layered on top of this module.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use sha2::{Digest as Sha2Digest, Sha256};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Failures raised while parsing, resolving, or digesting isolation values.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IsolationError {
    /// A capability string was neither a known `CAP_*` name nor `CAP_<decimal>`.
    #[error("unknown capability name: {0}")]
    UnknownCapabilityName(String),

    /// A requested capability ID exceeds the running kernel's `cap_last_cap`.
    #[error("capability id {requested} exceeds the running kernel's cap_last_cap ({cap_last_cap})")]
    CapabilityAboveCapLastCap {
        /// The capability ID that was requested.
        requested: u16,
        /// The running kernel's highest known capability ID.
        cap_last_cap: u16,
    },

    /// A capability addition was requested but the gate denies all additions.
    #[error("capability addition {0} denied: gate forbids all additions")]
    CapabilityAdditionDenied(LinuxCapability),

    /// A capability addition was requested but is not in the gate's allow-list.
    #[error("capability addition {0} not in the allow-list")]
    CapabilityAdditionNotAllowed(LinuxCapability),

    /// A digest payload contained a floating-point value, which the canonical
    /// JSON encoding forbids.
    #[error("float values are forbidden in isolation digest payloads")]
    FloatNotAllowed,

    /// A digest payload contained an explicit `null`, which the canonical
    /// JSON encoding forbids.
    #[error("explicit null values are forbidden in isolation digest payloads")]
    NullNotAllowed,

    /// The payload could not be converted to `serde_json::Value` for canonical
    /// encoding.
    #[error("failed to serialize digest payload: {0}")]
    DigestSerialization(String),

    /// A digest string did not match `sha256:<64 lowercase hex characters>`.
    #[error("malformed isolation digest string: {0}")]
    MalformedDigest(String),

    /// The profile's `schema_version` is not one this build understands.
    ///
    /// Unknown versions fail closed rather than falling back to a default.
    #[error("unknown isolation profile schema version {0}")]
    UnknownSchemaVersion(u32),

    /// `standard-v1` was constructed with at least one degraded control.
    ///
    /// `standard-v1` has no degraded control state; a degradation there is a
    /// contradiction rather than a value to accept.
    #[error("standard-v1 profile must not contain degraded controls")]
    StandardV1MustNotDegrade,

    /// `compatibility-v1` was constructed without an operator authorization.
    #[error("compatibility-v1 profile requires operator authorization")]
    CompatibilityRequiresAuthorization,

    /// A `compatibility-v1` degraded control carried an empty reason.
    #[error("compatibility-v1 degraded control missing a reason")]
    DegradationMissingReason,
}

// ---------------------------------------------------------------------------
// Profile and control identifiers
// ---------------------------------------------------------------------------

/// A built-in, versioned isolation profile.
///
/// Both this type and [`IsolationControl`] serialize with stable kebab-case
/// names so a schema inspection or wire payload is self-describing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationProfileId {
    /// The global default. No degraded controls are ever permitted.
    #[default]
    StandardV1,
    /// An explicit, operator-authorized escape hatch. Requires caller
    /// selection and an [`OperatorAuthorizationId`].
    CompatibilityV1,
}

/// One of the nine isolation concerns every resolved profile must address
/// exactly once.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationControl {
    /// Process identity (host-root vs. mapped/equivalent boundary).
    Identity,
    /// Process namespace boundary.
    Process,
    /// Mount namespace boundary and host mount allowlist.
    Mount,
    /// Network namespace boundary.
    Network,
    /// Linux capability set.
    LinuxCapabilities,
    /// `no_new_privileges`.
    NoNewPrivileges,
    /// Syscall filtering (seccomp or equivalent).
    SyscallFilter,
    /// Filesystem restrictions (device nodes, setid bits).
    Filesystem,
    /// Resource limits.
    Resources,
}

impl IsolationControl {
    /// Every control, in stable order.
    pub const ALL: [Self; 9] = [
        Self::Identity,
        Self::Process,
        Self::Mount,
        Self::Network,
        Self::LinuxCapabilities,
        Self::NoNewPrivileges,
        Self::SyscallFilter,
        Self::Filesystem,
        Self::Resources,
    ];
}

// ---------------------------------------------------------------------------
// Linux capability vocabulary
// ---------------------------------------------------------------------------

/// Table of known `CAP_*` names to their numeric kernel capability IDs.
///
/// Source: Linux `include/uapi/linux/capability.h`. This table only governs
/// human-facing name parsing and display; every digest projection uses the
/// numeric ID (see [`compute_isolation_digest`]), so adding a name here can
/// never change a historical digest.
const CAPABILITY_NAMES: &[(&str, u16)] = &[
    ("CAP_CHOWN", 0),
    ("CAP_DAC_OVERRIDE", 1),
    ("CAP_DAC_READ_SEARCH", 2),
    ("CAP_FOWNER", 3),
    ("CAP_FSETID", 4),
    ("CAP_KILL", 5),
    ("CAP_SETGID", 6),
    ("CAP_SETUID", 7),
    ("CAP_SETPCAP", 8),
    ("CAP_LINUX_IMMUTABLE", 9),
    ("CAP_NET_BIND_SERVICE", 10),
    ("CAP_NET_BROADCAST", 11),
    ("CAP_NET_ADMIN", 12),
    ("CAP_NET_RAW", 13),
    ("CAP_IPC_LOCK", 14),
    ("CAP_IPC_OWNER", 15),
    ("CAP_SYS_MODULE", 16),
    ("CAP_SYS_RAWIO", 17),
    ("CAP_SYS_CHROOT", 18),
    ("CAP_SYS_PTRACE", 19),
    ("CAP_SYS_PACCT", 20),
    ("CAP_SYS_ADMIN", 21),
    ("CAP_SYS_BOOT", 22),
    ("CAP_SYS_NICE", 23),
    ("CAP_SYS_RESOURCE", 24),
    ("CAP_SYS_TIME", 25),
    ("CAP_SYS_TTY_CONFIG", 26),
    ("CAP_MKNOD", 27),
    ("CAP_LEASE", 28),
    ("CAP_AUDIT_WRITE", 29),
    ("CAP_AUDIT_CONTROL", 30),
    ("CAP_SETFCAP", 31),
    ("CAP_MAC_OVERRIDE", 32),
    ("CAP_MAC_ADMIN", 33),
    ("CAP_SYSLOG", 34),
    ("CAP_WAKE_ALARM", 35),
    ("CAP_BLOCK_SUSPEND", 36),
    ("CAP_AUDIT_READ", 37),
    ("CAP_PERFMON", 38),
    ("CAP_BPF", 39),
    ("CAP_CHECKPOINT_RESTORE", 40),
];

/// The 14 capabilities Docker grants by default, pinned by numeric ID.
///
/// Changing this set requires a new baseline and profile version per the
/// design contract, so it is a fixed array rather than derived at runtime.
const DOCKER_DEFAULT_V1_CAP_IDS: [u16; 14] = [
    29, // CAP_AUDIT_WRITE
    0,  // CAP_CHOWN
    1,  // CAP_DAC_OVERRIDE
    3,  // CAP_FOWNER
    4,  // CAP_FSETID
    5,  // CAP_KILL
    27, // CAP_MKNOD
    10, // CAP_NET_BIND_SERVICE
    13, // CAP_NET_RAW
    31, // CAP_SETFCAP
    6,  // CAP_SETGID
    8,  // CAP_SETPCAP
    7,  // CAP_SETUID
    18, // CAP_SYS_CHROOT
];

/// A single Linux kernel capability, stored as its numeric ID.
///
/// Human-facing input accepts known `CAP_*` names and `CAP_<decimal>`. Wire
/// output may use names, but every digest projection uses the numeric ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LinuxCapability(u16);

impl LinuxCapability {
    /// Construct from a known numeric capability ID without name validation.
    #[must_use]
    pub const fn from_id(id: u16) -> Self {
        Self(id)
    }

    /// The numeric kernel capability ID.
    #[must_use]
    pub const fn id(self) -> u16 {
        self.0
    }

    /// The canonical `CAP_*` name, if this ID is in the known table.
    #[must_use]
    pub fn name(self) -> Option<&'static str> {
        CAPABILITY_NAMES
            .iter()
            .find(|(_, id)| *id == self.0)
            .map(|(name, _)| *name)
    }

    /// Parse a `CAP_*` name or a `CAP_<decimal>` numeric form.
    ///
    /// # Errors
    ///
    /// Returns [`IsolationError::UnknownCapabilityName`] if `input` is
    /// neither a known name nor a valid `CAP_<decimal>` form.
    pub fn parse(input: &str) -> Result<Self, IsolationError> {
        if let Some((_, id)) = CAPABILITY_NAMES.iter().find(|(name, _)| *name == input) {
            return Ok(Self(*id));
        }
        if let Some(digits) = input.strip_prefix("CAP_") {
            if let Ok(id) = digits.parse::<u16>() {
                return Ok(Self(id));
            }
        }
        Err(IsolationError::UnknownCapabilityName(input.to_string()))
    }
}

impl fmt::Display for LinuxCapability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.name() {
            Some(name) => write!(f, "{name}"),
            None => write!(f, "CAP_{}", self.0),
        }
    }
}

/// A named set of starting Linux capabilities.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinuxCapabilityBaseline {
    /// The 14-capability Docker default set.
    DockerDefaultV1,
}

impl LinuxCapabilityBaseline {
    /// The capabilities this baseline starts with.
    #[must_use]
    pub fn capabilities(self) -> BTreeSet<LinuxCapability> {
        match self {
            Self::DockerDefaultV1 => DOCKER_DEFAULT_V1_CAP_IDS
                .iter()
                .map(|&id| LinuxCapability(id))
                .collect(),
        }
    }
}

/// A caller's requested capability additions and drops.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LinuxCapabilityRequest {
    /// Capabilities to add on top of the baseline, subject to the gate.
    #[serde(default)]
    pub add: BTreeSet<LinuxCapability>,
    /// Capabilities to drop. Always honored; drops win conflicts.
    #[serde(default)]
    pub drop: BTreeSet<LinuxCapability>,
}

/// Governs which capability additions a [`LinuxCapabilityRequest`] may make.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum LinuxCapabilityGate {
    /// No additions are permitted; only drops may change the baseline.
    DenyAllAdditions,
    /// Only the listed capabilities may be added.
    AllowListed(BTreeSet<LinuxCapability>),
    /// Any addition is permitted (subject to `cap_last_cap`).
    AllowAll,
}

/// The resolved Linux capability sets for a container.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResolvedLinuxCapabilities {
    /// The effective set after baseline, authorized additions, and drops.
    pub effective: BTreeSet<LinuxCapability>,
    /// The inheritable set. Always empty under `standard-v1`.
    pub inheritable: BTreeSet<LinuxCapability>,
    /// The ambient set. Always empty under `standard-v1`.
    pub ambient: BTreeSet<LinuxCapability>,
    /// Degradation reason, if this control is degraded under
    /// `compatibility-v1`. Always `None` under `standard-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<Degraded>,
}

/// Resolve a baseline, a gate, and a caller request into an effective
/// capability set.
///
/// Resolution order: baseline, then authorized additions (checked against
/// `gate` and `cap_last_cap`), then drops. Drops always win conflicts.
///
/// # Errors
///
/// Returns an error if an addition is above `cap_last_cap`, or is denied or
/// not allow-listed by `gate`. Drops are never rejected.
pub fn resolve_linux_capabilities(
    baseline: LinuxCapabilityBaseline,
    gate: &LinuxCapabilityGate,
    request: &LinuxCapabilityRequest,
    cap_last_cap: u16,
) -> Result<ResolvedLinuxCapabilities, IsolationError> {
    for cap in &request.add {
        if cap.id() > cap_last_cap {
            return Err(IsolationError::CapabilityAboveCapLastCap {
                requested: cap.id(),
                cap_last_cap,
            });
        }
        match gate {
            LinuxCapabilityGate::DenyAllAdditions => {
                return Err(IsolationError::CapabilityAdditionDenied(*cap));
            }
            LinuxCapabilityGate::AllowListed(allowed) => {
                if !allowed.contains(cap) {
                    return Err(IsolationError::CapabilityAdditionNotAllowed(*cap));
                }
            }
            LinuxCapabilityGate::AllowAll => {}
        }
    }

    let mut effective = baseline.capabilities();
    effective.extend(request.add.iter().copied());
    for cap in &request.drop {
        effective.remove(cap);
    }

    Ok(ResolvedLinuxCapabilities {
        effective,
        inheritable: BTreeSet::new(),
        ambient: BTreeSet::new(),
        degraded: None,
    })
}

// ---------------------------------------------------------------------------
// Digest contract
// ---------------------------------------------------------------------------

/// The domain a value is digested under.
///
/// Each variant name encodes its schema version (e.g. `WorkloadV2`); the
/// numeric suffix is split out from the kebab-case domain name used in the
/// digest input (see [`compute_isolation_digest`]) so the two are hashed as
/// independent, explicitly length-prefixed fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IsolationDigestDomain {
    /// A workload's execution manifest.
    WorkloadV2,
    /// A resolved isolation profile.
    ProfileV1,
    /// An isolation request DTO.
    RequestV1,
    /// A staged runtime plan.
    PlanV1,
    /// A binding between a plan and an adapter environment.
    BindingV1,
    /// Staged enforcement evidence.
    EvidenceV1,
    /// Cleanup-ownership evidence.
    CleanupV1,
    /// An adapter environment generation marker.
    EnvironmentGenerationV1,
}

impl IsolationDigestDomain {
    /// The kebab-case domain name, without the version suffix.
    const fn name(self) -> &'static str {
        match self {
            Self::WorkloadV2 => "workload",
            Self::ProfileV1 => "profile",
            Self::RequestV1 => "request",
            Self::PlanV1 => "plan",
            Self::BindingV1 => "binding",
            Self::EvidenceV1 => "evidence",
            Self::CleanupV1 => "cleanup",
            Self::EnvironmentGenerationV1 => "environment-generation",
        }
    }

    /// The schema version encoded in this domain's variant name.
    const fn schema_version(self) -> u32 {
        match self {
            Self::WorkloadV2 => 2,
            Self::ProfileV1
            | Self::RequestV1
            | Self::PlanV1
            | Self::BindingV1
            | Self::EvidenceV1
            | Self::CleanupV1
            | Self::EnvironmentGenerationV1 => 1,
        }
    }
}

/// A SHA-256 digest over a canonical JSON projection of an isolation value.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct IsolationDigest([u8; 32]);

impl fmt::Display for IsolationDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sha256:{}", hex::encode(self.0))
    }
}

impl fmt::Debug for IsolationDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "IsolationDigest({self})")
    }
}

impl FromStr for IsolationDigest {
    type Err = IsolationError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex_part = s
            .strip_prefix("sha256:")
            .ok_or_else(|| IsolationError::MalformedDigest(s.to_string()))?;
        if hex_part.len() != 64
            || !hex_part
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err(IsolationError::MalformedDigest(s.to_string()));
        }
        let bytes =
            hex::decode(hex_part).map_err(|_| IsolationError::MalformedDigest(s.to_string()))?;
        let array: [u8; 32] = bytes
            .try_into()
            .map_err(|_| IsolationError::MalformedDigest(s.to_string()))?;
        Ok(Self(array))
    }
}

impl Serialize for IsolationDigest {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for IsolationDigest {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::from_str(&raw).map_err(serde::de::Error::custom)
    }
}

/// Recursively encode a [`serde_json::Value`] into the canonical byte form
/// used by [`compute_isolation_digest`].
///
/// Object keys are already sorted by UTF-8 byte order because
/// `serde_json::Value::Object` is backed by a `BTreeMap` in this workspace
/// (the `preserve_order` feature is not enabled); this function does not
/// re-sort them, it only forbids floats and explicit nulls and emits no
/// insignificant whitespace.
fn canonicalize_json(value: &serde_json::Value, out: &mut Vec<u8>) -> Result<(), IsolationError> {
    match value {
        serde_json::Value::Null => Err(IsolationError::NullNotAllowed),
        serde_json::Value::Bool(b) => {
            out.extend_from_slice(if *b { b"true" } else { b"false" });
            Ok(())
        }
        serde_json::Value::Number(n) => {
            if n.is_f64() {
                return Err(IsolationError::FloatNotAllowed);
            }
            out.extend_from_slice(n.to_string().as_bytes());
            Ok(())
        }
        serde_json::Value::String(s) => {
            let encoded = serde_json::to_string(s)
                .map_err(|e| IsolationError::DigestSerialization(e.to_string()))?;
            out.extend_from_slice(encoded.as_bytes());
            Ok(())
        }
        serde_json::Value::Array(items) => {
            out.push(b'[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                canonicalize_json(item, out)?;
            }
            out.push(b']');
            Ok(())
        }
        serde_json::Value::Object(map) => {
            out.push(b'{');
            for (i, (key, val)) in map.iter().enumerate() {
                if i > 0 {
                    out.push(b',');
                }
                let key_encoded = serde_json::to_string(key)
                    .map_err(|e| IsolationError::DigestSerialization(e.to_string()))?;
                out.extend_from_slice(key_encoded.as_bytes());
                out.push(b':');
                canonicalize_json(val, out)?;
            }
            out.push(b'}');
            Ok(())
        }
    }
}

/// Serialize `value` to the canonical JSON byte form.
///
/// # Errors
///
/// Returns an error if `value` cannot be converted to `serde_json::Value`, or
/// if its projection contains a float or an explicit `null`.
fn canonical_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, IsolationError> {
    let json_value = serde_json::to_value(value)
        .map_err(|e| IsolationError::DigestSerialization(e.to_string()))?;
    let mut out = Vec::new();
    canonicalize_json(&json_value, &mut out)?;
    Ok(out)
}

/// Compute the deterministic isolation digest of `value` under `domain`.
///
/// The algorithm is SHA-256 over: an ASCII prefix `minibox-isolation` and one
/// NUL byte; the domain name's length (big-endian `u32`) and bytes; the
/// domain's schema version (big-endian `u32`); the canonical payload's length
/// (big-endian `u64`); and the canonical JSON payload bytes.
///
/// # Errors
///
/// Returns an error if `value`'s canonical JSON projection contains a float
/// or an explicit `null`, or cannot be produced at all.
pub fn compute_isolation_digest<T: Serialize>(
    domain: IsolationDigestDomain,
    value: &T,
) -> Result<IsolationDigest, IsolationError> {
    let payload = canonical_json_bytes(value)?;
    let name = domain.name();

    let mut hasher = Sha256::new();
    hasher.update(b"minibox-isolation");
    hasher.update([0u8]);
    hasher.update(u32::try_from(name.len()).unwrap_or(u32::MAX).to_be_bytes());
    hasher.update(name.as_bytes());
    hasher.update(domain.schema_version().to_be_bytes());
    hasher.update(
        u64::try_from(payload.len())
            .unwrap_or(u64::MAX)
            .to_be_bytes(),
    );
    hasher.update(&payload);

    let digest: [u8; 32] = hasher.finalize().into();
    Ok(IsolationDigest(digest))
}

// ---------------------------------------------------------------------------
// Per-control policy types
// ---------------------------------------------------------------------------

/// A reason recorded against a degraded control under `compatibility-v1`.
///
/// `standard-v1` controls never carry one. Every degradation is bound to the
/// profile's single [`OperatorAuthorizationId`] rather than carrying its own,
/// since `compatibility-v1` requires exactly one authorization per profile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Degraded {
    /// Human-readable explanation of why this control is degraded.
    pub reason: String,
}

/// An opaque identifier for an operator's authorization of a
/// `compatibility-v1` profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct OperatorAuthorizationId(String);

impl OperatorAuthorizationId {
    /// Wrap an existing authorization identifier.
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    /// The identifier's string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Where a control's enforcement evidence authority comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceAuthority {
    /// The profile itself dictates what counts as valid evidence.
    Profile,
    /// The adapter selects what counts as valid evidence.
    Adapter,
}

/// The process identity boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum IdentityBoundary {
    /// A non-root identity mapped by the kernel user namespace.
    MappedNonRoot,
    /// A non-mapped boundary judged equivalent to a mapped one.
    EquivalentBoundary,
    /// Host-root identity. Forbidden under `standard-v1`.
    HostRoot,
}

/// Resolved identity control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityPolicy {
    /// The identity boundary in effect.
    pub boundary: IdentityBoundary,
    /// Degradation reason, if degraded under `compatibility-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<Degraded>,
}

/// The process namespace boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ProcessBoundary {
    /// A process namespace boundary is required.
    Required,
    /// No process namespace boundary. Only valid as a degradation.
    None,
}

/// Resolved process control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessPolicy {
    /// The process boundary in effect.
    pub boundary: ProcessBoundary,
    /// Degradation reason, if degraded under `compatibility-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<Degraded>,
}

/// Access mode for an allow-listed host mount.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum MountAccessMode {
    /// Read-only access.
    ReadOnly,
    /// Read-write access.
    ReadWrite,
}

/// One operator-allowlisted host mount.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostMountRule {
    /// The validated host path.
    pub host_path: String,
    /// The access mode granted for this path.
    pub access: MountAccessMode,
}

/// Resolved mount control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MountPolicy {
    /// The operator-allowlisted host mounts, if any.
    pub host_mounts: Vec<HostMountRule>,
    /// Degradation reason, if degraded under `compatibility-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<Degraded>,
}

/// The network namespace boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkBoundary {
    /// A non-host network boundary (e.g. bridge, none) is required.
    NonHost,
    /// Host network namespace sharing. Only valid as a degradation.
    Host,
}

/// Resolved network control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    /// The network boundary in effect.
    pub boundary: NetworkBoundary,
    /// Degradation reason, if degraded under `compatibility-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<Degraded>,
}

/// Resolved `no_new_privileges` control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoNewPrivilegesPolicy {
    /// Whether `no_new_privileges` is required. Always `true` under
    /// `standard-v1`.
    pub required: bool,
    /// Degradation reason, if degraded under `compatibility-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<Degraded>,
}

/// Which syscall-filtering artifact a profile requires.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SyscallFilterArtifact {
    /// The `standard-v1` mount-immutability filter.
    StandardV1MountImmutability,
    /// No syscall filter. Only valid as a degradation.
    None,
}

/// Resolved syscall filter control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyscallFilterPolicy {
    /// The required filter artifact.
    pub artifact: SyscallFilterArtifact,
    /// Degradation reason, if degraded under `compatibility-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<Degraded>,
}

/// Resolved filesystem control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FilesystemPolicy {
    /// Whether device node creation is denied. Always `true` under
    /// `standard-v1`.
    pub device_nodes_denied: bool,
    /// Whether setuid/setgid bits are denied. Always `true` under
    /// `standard-v1`.
    pub setid_bits_denied: bool,
    /// Degradation reason, if degraded under `compatibility-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<Degraded>,
}

/// A caller's requested resource limits, prior to profile resolution.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceRequest {
    /// Requested PID limit. Defaults to 1024 under `standard-v1` if absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pids_max: Option<u32>,
    /// Requested memory limit in bytes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit_bytes: Option<u64>,
    /// Requested CPU weight.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_weight: Option<u32>,
}

/// Resolved resource control.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourcePolicy {
    /// The enforced PID limit.
    pub pids_max: u32,
    /// The enforced memory limit in bytes, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_limit_bytes: Option<u64>,
    /// The enforced CPU weight, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cpu_weight: Option<u32>,
    /// Degradation reason, if degraded under `compatibility-v1`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub degraded: Option<Degraded>,
}

/// The default PID limit `standard-v1` enforces when a caller requests none.
const STANDARD_V1_DEFAULT_PIDS_MAX: u32 = 1024;

/// The schema version this build emits and accepts for
/// [`ResolvedIsolationProfile`]. Any other value fails closed.
const CURRENT_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Resolved profile
// ---------------------------------------------------------------------------

/// A fully resolved, deterministic isolation profile.
///
/// Fields are private. The only ways to construct one are [`standard_v1`] and
/// [`compatibility_v1`] (construction) and this type's [`Deserialize`] impl
/// (trusted deserialization) — both paths share one validating constructor.
/// Callers get read-only accessors.
///
/// A valid resolved profile contains each of the nine [`IsolationControl`]
/// values exactly once; this is guaranteed by the struct's shape (one field
/// per control) rather than checked at runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedIsolationProfile {
    schema_version: u32,
    profile: IsolationProfileId,
    identity: IdentityPolicy,
    process: ProcessPolicy,
    mount: MountPolicy,
    network: NetworkPolicy,
    capabilities: ResolvedLinuxCapabilities,
    no_new_privileges: NoNewPrivilegesPolicy,
    syscall_filter: SyscallFilterPolicy,
    filesystem: FilesystemPolicy,
    resources: ResourcePolicy,
    allowed_evidence_authorities: BTreeMap<IsolationControl, BTreeSet<EvidenceAuthority>>,
    policy_digest: IsolationDigest,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operator_authorization: Option<OperatorAuthorizationId>,
}

/// Mirrors [`ResolvedIsolationProfile`]'s fields for digesting and for
/// trusted deserialization, prior to validation.
#[derive(Serialize, Deserialize)]
struct ResolvedIsolationProfileFields {
    schema_version: u32,
    profile: IsolationProfileId,
    identity: IdentityPolicy,
    process: ProcessPolicy,
    mount: MountPolicy,
    network: NetworkPolicy,
    capabilities: ResolvedLinuxCapabilities,
    no_new_privileges: NoNewPrivilegesPolicy,
    syscall_filter: SyscallFilterPolicy,
    filesystem: FilesystemPolicy,
    resources: ResourcePolicy,
    allowed_evidence_authorities: BTreeMap<IsolationControl, BTreeSet<EvidenceAuthority>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    operator_authorization: Option<OperatorAuthorizationId>,
}

impl ResolvedIsolationProfile {
    /// The schema version this profile was resolved under.
    #[must_use]
    pub const fn schema_version(&self) -> u32 {
        self.schema_version
    }

    /// Which built-in profile this is.
    #[must_use]
    pub const fn profile(&self) -> IsolationProfileId {
        self.profile
    }

    /// Resolved identity control.
    #[must_use]
    pub const fn identity(&self) -> &IdentityPolicy {
        &self.identity
    }

    /// Resolved process control.
    #[must_use]
    pub const fn process(&self) -> &ProcessPolicy {
        &self.process
    }

    /// Resolved mount control.
    #[must_use]
    pub const fn mount(&self) -> &MountPolicy {
        &self.mount
    }

    /// Resolved network control.
    #[must_use]
    pub const fn network(&self) -> &NetworkPolicy {
        &self.network
    }

    /// Resolved Linux capability control.
    #[must_use]
    pub const fn capabilities(&self) -> &ResolvedLinuxCapabilities {
        &self.capabilities
    }

    /// Resolved `no_new_privileges` control.
    #[must_use]
    pub const fn no_new_privileges(&self) -> &NoNewPrivilegesPolicy {
        &self.no_new_privileges
    }

    /// Resolved syscall filter control.
    #[must_use]
    pub const fn syscall_filter(&self) -> &SyscallFilterPolicy {
        &self.syscall_filter
    }

    /// Resolved filesystem control.
    #[must_use]
    pub const fn filesystem(&self) -> &FilesystemPolicy {
        &self.filesystem
    }

    /// Resolved resource control.
    #[must_use]
    pub const fn resources(&self) -> &ResourcePolicy {
        &self.resources
    }

    /// The evidence authority allowed for each control.
    #[must_use]
    pub const fn allowed_evidence_authorities(
        &self,
    ) -> &BTreeMap<IsolationControl, BTreeSet<EvidenceAuthority>> {
        &self.allowed_evidence_authorities
    }

    /// The deterministic digest of this profile's policy content.
    #[must_use]
    pub const fn policy_digest(&self) -> IsolationDigest {
        self.policy_digest
    }

    /// The operator authorization binding, if this is a `compatibility-v1`
    /// profile.
    #[must_use]
    pub const fn operator_authorization(&self) -> Option<&OperatorAuthorizationId> {
        self.operator_authorization.as_ref()
    }

    /// Whether any control in this profile is degraded.
    #[must_use]
    pub const fn has_degraded_controls(&self) -> bool {
        self.identity.degraded.is_some()
            || self.process.degraded.is_some()
            || self.mount.degraded.is_some()
            || self.network.degraded.is_some()
            || self.capabilities.degraded.is_some()
            || self.no_new_privileges.degraded.is_some()
            || self.syscall_filter.degraded.is_some()
            || self.filesystem.degraded.is_some()
            || self.resources.degraded.is_some()
    }

    /// The one validating constructor shared by [`standard_v1`],
    /// [`compatibility_v1`], and trusted deserialization.
    ///
    /// # Errors
    ///
    /// Returns an error if `fields.schema_version` is unknown, if
    /// `fields.profile` is `StandardV1` and any control is degraded, or if
    /// `fields.profile` is `CompatibilityV1` and no operator authorization is
    /// present, or a degraded control carries an empty reason.
    fn validate_and_build(fields: ResolvedIsolationProfileFields) -> Result<Self, IsolationError> {
        if fields.schema_version != CURRENT_SCHEMA_VERSION {
            return Err(IsolationError::UnknownSchemaVersion(fields.schema_version));
        }

        let degraded_reasons: [Option<&Degraded>; 9] = [
            fields.identity.degraded.as_ref(),
            fields.process.degraded.as_ref(),
            fields.mount.degraded.as_ref(),
            fields.network.degraded.as_ref(),
            fields.capabilities.degraded.as_ref(),
            fields.no_new_privileges.degraded.as_ref(),
            fields.syscall_filter.degraded.as_ref(),
            fields.filesystem.degraded.as_ref(),
            fields.resources.degraded.as_ref(),
        ];
        let any_degraded = degraded_reasons.iter().any(Option::is_some);

        match fields.profile {
            IsolationProfileId::StandardV1 => {
                if any_degraded {
                    return Err(IsolationError::StandardV1MustNotDegrade);
                }
            }
            IsolationProfileId::CompatibilityV1 => {
                if fields.operator_authorization.is_none() {
                    return Err(IsolationError::CompatibilityRequiresAuthorization);
                }
                for degraded in degraded_reasons.into_iter().flatten() {
                    if degraded.reason.trim().is_empty() {
                        return Err(IsolationError::DegradationMissingReason);
                    }
                }
            }
        }

        let policy_digest = compute_isolation_digest(IsolationDigestDomain::ProfileV1, &fields)?;

        Ok(Self {
            schema_version: fields.schema_version,
            profile: fields.profile,
            identity: fields.identity,
            process: fields.process,
            mount: fields.mount,
            network: fields.network,
            capabilities: fields.capabilities,
            no_new_privileges: fields.no_new_privileges,
            syscall_filter: fields.syscall_filter,
            filesystem: fields.filesystem,
            resources: fields.resources,
            allowed_evidence_authorities: fields.allowed_evidence_authorities,
            policy_digest,
            operator_authorization: fields.operator_authorization,
        })
    }
}

impl<'de> Deserialize<'de> for ResolvedIsolationProfile {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let fields = ResolvedIsolationProfileFields::deserialize(deserializer)?;
        Self::validate_and_build(fields).map_err(serde::de::Error::custom)
    }
}

/// Resolve the `standard-v1` profile: the global default with no degraded
/// controls.
///
/// Per the design's "Built-In Semantics": host-root identity is forbidden;
/// process, mount, and non-host network boundaries are required; the Docker
/// capability baseline applies with `request`/`gate` authorizing any
/// modification, and the inheritable/ambient sets are always empty;
/// `no_new_privileges` is required; the `standard-v1` mount-immutability
/// syscall filter is required; device nodes and setid bits are denied; host
/// mounts follow `host_mounts`; requested resource limits are enforced with
/// `pids_max` defaulting to 1024; evidence authority is profile-owned for
/// every control.
///
/// # Errors
///
/// Returns an error if capability resolution fails (see
/// [`resolve_linux_capabilities`]).
pub fn standard_v1(
    host_mounts: Vec<HostMountRule>,
    capability_request: &LinuxCapabilityRequest,
    capability_gate: &LinuxCapabilityGate,
    cap_last_cap: u16,
    resources: ResourceRequest,
) -> Result<ResolvedIsolationProfile, IsolationError> {
    let capabilities = resolve_linux_capabilities(
        LinuxCapabilityBaseline::DockerDefaultV1,
        capability_gate,
        capability_request,
        cap_last_cap,
    )?;

    let allowed_evidence_authorities = IsolationControl::ALL
        .into_iter()
        .map(|control| (control, BTreeSet::from([EvidenceAuthority::Profile])))
        .collect();

    let fields = ResolvedIsolationProfileFields {
        schema_version: CURRENT_SCHEMA_VERSION,
        profile: IsolationProfileId::StandardV1,
        identity: IdentityPolicy {
            boundary: IdentityBoundary::MappedNonRoot,
            degraded: None,
        },
        process: ProcessPolicy {
            boundary: ProcessBoundary::Required,
            degraded: None,
        },
        mount: MountPolicy {
            host_mounts,
            degraded: None,
        },
        network: NetworkPolicy {
            boundary: NetworkBoundary::NonHost,
            degraded: None,
        },
        capabilities,
        no_new_privileges: NoNewPrivilegesPolicy {
            required: true,
            degraded: None,
        },
        syscall_filter: SyscallFilterPolicy {
            artifact: SyscallFilterArtifact::StandardV1MountImmutability,
            degraded: None,
        },
        filesystem: FilesystemPolicy {
            device_nodes_denied: true,
            setid_bits_denied: true,
            degraded: None,
        },
        resources: ResourcePolicy {
            pids_max: resources.pids_max.unwrap_or(STANDARD_V1_DEFAULT_PIDS_MAX),
            memory_limit_bytes: resources.memory_limit_bytes,
            cpu_weight: resources.cpu_weight,
            degraded: None,
        },
        allowed_evidence_authorities,
        operator_authorization: None,
    };

    ResolvedIsolationProfile::validate_and_build(fields)
}

/// Resolve the `compatibility-v1` profile: an explicit, operator-authorized
/// escape hatch.
///
/// `degradations` names which of the nine controls are degraded from their
/// `standard-v1` values and why; every entry must carry a non-empty reason.
/// Every degraded control's evidence authority becomes adapter-selected
/// rather than profile-owned, since `standard-v1`'s "profile-owned, not
/// adapter-selected" guarantee does not extend to an acknowledged
/// degradation.
///
/// # Errors
///
/// Returns an error if capability resolution fails, or if any entry in
/// `degradations` carries an empty reason.
pub fn compatibility_v1(
    operator_authorization: OperatorAuthorizationId,
    degradations: BTreeMap<IsolationControl, Degraded>,
    host_mounts: Vec<HostMountRule>,
    capability_request: &LinuxCapabilityRequest,
    capability_gate: &LinuxCapabilityGate,
    cap_last_cap: u16,
    resources: ResourceRequest,
) -> Result<ResolvedIsolationProfile, IsolationError> {
    let mut capabilities = resolve_linux_capabilities(
        LinuxCapabilityBaseline::DockerDefaultV1,
        capability_gate,
        capability_request,
        cap_last_cap,
    )?;
    capabilities.degraded = degradations
        .get(&IsolationControl::LinuxCapabilities)
        .cloned();

    let allowed_evidence_authorities = IsolationControl::ALL
        .into_iter()
        .map(|control| {
            let authority = if degradations.contains_key(&control) {
                EvidenceAuthority::Adapter
            } else {
                EvidenceAuthority::Profile
            };
            (control, BTreeSet::from([authority]))
        })
        .collect();

    let fields = ResolvedIsolationProfileFields {
        schema_version: CURRENT_SCHEMA_VERSION,
        profile: IsolationProfileId::CompatibilityV1,
        identity: IdentityPolicy {
            boundary: IdentityBoundary::MappedNonRoot,
            degraded: degradations.get(&IsolationControl::Identity).cloned(),
        },
        process: ProcessPolicy {
            boundary: ProcessBoundary::Required,
            degraded: degradations.get(&IsolationControl::Process).cloned(),
        },
        mount: MountPolicy {
            host_mounts,
            degraded: degradations.get(&IsolationControl::Mount).cloned(),
        },
        network: NetworkPolicy {
            boundary: NetworkBoundary::NonHost,
            degraded: degradations.get(&IsolationControl::Network).cloned(),
        },
        capabilities,
        no_new_privileges: NoNewPrivilegesPolicy {
            required: true,
            degraded: degradations
                .get(&IsolationControl::NoNewPrivileges)
                .cloned(),
        },
        syscall_filter: SyscallFilterPolicy {
            artifact: SyscallFilterArtifact::StandardV1MountImmutability,
            degraded: degradations.get(&IsolationControl::SyscallFilter).cloned(),
        },
        filesystem: FilesystemPolicy {
            device_nodes_denied: true,
            setid_bits_denied: true,
            degraded: degradations.get(&IsolationControl::Filesystem).cloned(),
        },
        resources: ResourcePolicy {
            pids_max: resources.pids_max.unwrap_or(STANDARD_V1_DEFAULT_PIDS_MAX),
            memory_limit_bytes: resources.memory_limit_bytes,
            cpu_weight: resources.cpu_weight,
            degraded: degradations.get(&IsolationControl::Resources).cloned(),
        },
        allowed_evidence_authorities,
        operator_authorization: Some(operator_authorization),
    };

    ResolvedIsolationProfile::validate_and_build(fields)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    // -- profile id / control enums ----------------------------------------

    #[test]
    fn profile_id_uses_pinned_kebab_case_names() {
        assert_eq!(
            serde_json::to_string(&IsolationProfileId::StandardV1).unwrap(),
            "\"standard-v1\""
        );
        assert_eq!(
            serde_json::to_string(&IsolationProfileId::CompatibilityV1).unwrap(),
            "\"compatibility-v1\""
        );
    }

    #[test]
    fn isolation_control_uses_pinned_kebab_case_names() {
        let pinned = [
            (IsolationControl::Identity, "\"identity\""),
            (IsolationControl::Process, "\"process\""),
            (IsolationControl::Mount, "\"mount\""),
            (IsolationControl::Network, "\"network\""),
            (
                IsolationControl::LinuxCapabilities,
                "\"linux-capabilities\"",
            ),
            (IsolationControl::NoNewPrivileges, "\"no-new-privileges\""),
            (IsolationControl::SyscallFilter, "\"syscall-filter\""),
            (IsolationControl::Filesystem, "\"filesystem\""),
            (IsolationControl::Resources, "\"resources\""),
        ];
        for (control, expected) in pinned {
            assert_eq!(serde_json::to_string(&control).unwrap(), expected);
        }
        assert_eq!(IsolationControl::ALL.len(), 9);
    }

    #[test]
    fn profile_id_roundtrips_json() {
        for id in [
            IsolationProfileId::StandardV1,
            IsolationProfileId::CompatibilityV1,
        ] {
            let json = serde_json::to_string(&id).unwrap();
            let decoded: IsolationProfileId = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, id);
        }
    }

    // -- capability parsing -------------------------------------------------

    #[test]
    fn docker_default_v1_pins_fourteen_capabilities() {
        let caps = LinuxCapabilityBaseline::DockerDefaultV1.capabilities();
        assert_eq!(caps.len(), 14);
        for name in [
            "CAP_AUDIT_WRITE",
            "CAP_CHOWN",
            "CAP_DAC_OVERRIDE",
            "CAP_FOWNER",
            "CAP_FSETID",
            "CAP_KILL",
            "CAP_MKNOD",
            "CAP_NET_BIND_SERVICE",
            "CAP_NET_RAW",
            "CAP_SETFCAP",
            "CAP_SETGID",
            "CAP_SETPCAP",
            "CAP_SETUID",
            "CAP_SYS_CHROOT",
        ] {
            let cap = LinuxCapability::parse(name).unwrap();
            assert!(
                caps.contains(&cap),
                "{name} ({cap}) missing from Docker default baseline"
            );
        }
    }

    #[test]
    fn capability_name_roundtrips_through_numeric_id() {
        let cap = LinuxCapability::parse("CAP_SYS_ADMIN").unwrap();
        assert_eq!(cap.id(), 21);
        assert_eq!(cap.name(), Some("CAP_SYS_ADMIN"));
        assert_eq!(cap.to_string(), "CAP_SYS_ADMIN");
    }

    #[test]
    fn capability_numeric_form_parses_without_a_name() {
        let cap = LinuxCapability::parse("CAP_41").unwrap();
        assert_eq!(cap.id(), 41);
        assert_eq!(cap.name(), None);
        assert_eq!(cap.to_string(), "CAP_41");
    }

    #[test]
    fn unknown_capability_name_is_rejected() {
        assert_eq!(
            LinuxCapability::parse("CAP_NOT_REAL"),
            Err(IsolationError::UnknownCapabilityName(
                "CAP_NOT_REAL".to_string()
            ))
        );
        assert_eq!(
            LinuxCapability::parse("not even cap shaped"),
            Err(IsolationError::UnknownCapabilityName(
                "not even cap shaped".to_string()
            ))
        );
    }

    #[test]
    fn capability_serializes_as_bare_numeric_id() {
        let cap = LinuxCapability::parse("CAP_CHOWN").unwrap();
        assert_eq!(serde_json::to_string(&cap).unwrap(), "0");
    }

    // -- capability resolution ----------------------------------------------

    #[test]
    fn drop_wins_even_against_baseline_and_add() {
        let chown = LinuxCapability::parse("CAP_CHOWN").unwrap();
        let request = LinuxCapabilityRequest {
            add: BTreeSet::from([chown]),
            drop: BTreeSet::from([chown]),
        };
        let resolved = resolve_linux_capabilities(
            LinuxCapabilityBaseline::DockerDefaultV1,
            &LinuxCapabilityGate::AllowAll,
            &request,
            63,
        )
        .unwrap();
        assert!(!resolved.effective.contains(&chown));
    }

    #[test]
    fn deny_all_additions_rejects_any_add() {
        let admin = LinuxCapability::parse("CAP_SYS_ADMIN").unwrap();
        let request = LinuxCapabilityRequest {
            add: BTreeSet::from([admin]),
            drop: BTreeSet::new(),
        };
        let err = resolve_linux_capabilities(
            LinuxCapabilityBaseline::DockerDefaultV1,
            &LinuxCapabilityGate::DenyAllAdditions,
            &request,
            63,
        )
        .unwrap_err();
        assert_eq!(err, IsolationError::CapabilityAdditionDenied(admin));
    }

    #[test]
    fn allow_listed_rejects_additions_outside_the_list() {
        let admin = LinuxCapability::parse("CAP_SYS_ADMIN").unwrap();
        let net_admin = LinuxCapability::parse("CAP_NET_ADMIN").unwrap();
        let request = LinuxCapabilityRequest {
            add: BTreeSet::from([admin]),
            drop: BTreeSet::new(),
        };
        let gate = LinuxCapabilityGate::AllowListed(BTreeSet::from([net_admin]));
        let err = resolve_linux_capabilities(
            LinuxCapabilityBaseline::DockerDefaultV1,
            &gate,
            &request,
            63,
        )
        .unwrap_err();
        assert_eq!(err, IsolationError::CapabilityAdditionNotAllowed(admin));
    }

    #[test]
    fn allow_listed_permits_additions_inside_the_list() {
        let net_admin = LinuxCapability::parse("CAP_NET_ADMIN").unwrap();
        let request = LinuxCapabilityRequest {
            add: BTreeSet::from([net_admin]),
            drop: BTreeSet::new(),
        };
        let gate = LinuxCapabilityGate::AllowListed(BTreeSet::from([net_admin]));
        let resolved = resolve_linux_capabilities(
            LinuxCapabilityBaseline::DockerDefaultV1,
            &gate,
            &request,
            63,
        )
        .unwrap();
        assert!(resolved.effective.contains(&net_admin));
    }

    #[test]
    fn addition_above_cap_last_cap_is_rejected() {
        let too_high = LinuxCapability::from_id(50);
        let request = LinuxCapabilityRequest {
            add: BTreeSet::from([too_high]),
            drop: BTreeSet::new(),
        };
        let err = resolve_linux_capabilities(
            LinuxCapabilityBaseline::DockerDefaultV1,
            &LinuxCapabilityGate::AllowAll,
            &request,
            40,
        )
        .unwrap_err();
        assert_eq!(
            err,
            IsolationError::CapabilityAboveCapLastCap {
                requested: 50,
                cap_last_cap: 40
            }
        );
    }

    #[test]
    fn resolved_capabilities_have_empty_inheritable_and_ambient_sets() {
        let resolved = resolve_linux_capabilities(
            LinuxCapabilityBaseline::DockerDefaultV1,
            &LinuxCapabilityGate::AllowAll,
            &LinuxCapabilityRequest::default(),
            63,
        )
        .unwrap();
        assert!(resolved.inheritable.is_empty());
        assert!(resolved.ambient.is_empty());
    }

    // -- digest contract ------------------------------------------------------

    #[derive(Serialize)]
    struct DigestFixture {
        b: u32,
        a: u32,
    }

    #[test]
    fn digest_is_deterministic_regardless_of_struct_field_declaration_order() {
        #[derive(Serialize)]
        struct Reordered {
            a: u32,
            b: u32,
        }
        let first = compute_isolation_digest(
            IsolationDigestDomain::ProfileV1,
            &DigestFixture { b: 2, a: 1 },
        )
        .unwrap();
        let second =
            compute_isolation_digest(IsolationDigestDomain::ProfileV1, &Reordered { a: 1, b: 2 })
                .unwrap();
        assert_eq!(first, second, "object key order must not affect the digest");
    }

    #[test]
    fn digest_set_order_does_not_affect_the_digest() {
        let one: BTreeSet<u32> = [1, 2, 3].into_iter().collect();
        let other: BTreeSet<u32> = [3, 2, 1].into_iter().collect();
        let first = compute_isolation_digest(IsolationDigestDomain::RequestV1, &one).unwrap();
        let second = compute_isolation_digest(IsolationDigestDomain::RequestV1, &other).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn digest_differs_across_domains_for_the_same_payload() {
        let payload = DigestFixture { a: 1, b: 2 };
        let profile = compute_isolation_digest(IsolationDigestDomain::ProfileV1, &payload).unwrap();
        let request = compute_isolation_digest(IsolationDigestDomain::RequestV1, &payload).unwrap();
        assert_ne!(profile, request);
    }

    #[test]
    fn digest_rejects_floats() {
        #[derive(Serialize)]
        struct WithFloat {
            value: f64,
        }
        let err =
            compute_isolation_digest(IsolationDigestDomain::ProfileV1, &WithFloat { value: 1.5 })
                .unwrap_err();
        assert_eq!(err, IsolationError::FloatNotAllowed);
    }

    #[test]
    fn digest_rejects_explicit_null() {
        #[derive(Serialize)]
        struct WithNull {
            value: Option<u32>,
        }
        let err =
            compute_isolation_digest(IsolationDigestDomain::ProfileV1, &WithNull { value: None })
                .unwrap_err();
        assert_eq!(err, IsolationError::NullNotAllowed);
    }

    #[test]
    fn digest_display_and_parse_roundtrip() {
        let digest = compute_isolation_digest(
            IsolationDigestDomain::ProfileV1,
            &DigestFixture { a: 1, b: 2 },
        )
        .unwrap();
        let text = digest.to_string();
        assert!(text.starts_with("sha256:"));
        assert_eq!(text.len(), "sha256:".len() + 64);
        let parsed: IsolationDigest = text.parse().unwrap();
        assert_eq!(parsed, digest);
    }

    #[test]
    fn digest_parse_rejects_malformed_strings() {
        for bad in [
            "not-a-digest",
            "sha256:",
            "sha256:00",
            "sha512:0000000000000000000000000000000000000000000000000000000000000000",
            &"sha256:AB".to_string().repeat(32)[..],
        ] {
            assert!(
                IsolationDigest::from_str(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
    }

    #[test]
    fn digest_golden_vector_for_profile_v1_domain() {
        // First-time pinning: this exact value was computed by this
        // implementation and then hardcoded here, per the design's
        // requirement for golden vectors. There is no external oracle for a
        // brand-new digest scheme; this test's purpose is to catch any
        // future accidental change to the algorithm, not to validate against
        // an independent reference.
        let digest = compute_isolation_digest(
            IsolationDigestDomain::ProfileV1,
            &DigestFixture { a: 1, b: 2 },
        )
        .unwrap();
        assert_eq!(
            digest.to_string(),
            "sha256:b726a38b21f28098b68b5841eb2b59bea39116acaaa84d20eac06dcf57841065"
        );
    }

    // -- resolved profile builders --------------------------------------------

    #[test]
    fn standard_v1_has_no_degraded_controls() {
        let profile = standard_v1(
            vec![],
            &LinuxCapabilityRequest::default(),
            &LinuxCapabilityGate::AllowAll,
            63,
            ResourceRequest::default(),
        )
        .unwrap();
        assert!(!profile.has_degraded_controls());
        assert_eq!(profile.profile(), IsolationProfileId::StandardV1);
        assert!(profile.operator_authorization().is_none());
    }

    #[test]
    fn standard_v1_defaults_pids_max_to_1024() {
        let profile = standard_v1(
            vec![],
            &LinuxCapabilityRequest::default(),
            &LinuxCapabilityGate::AllowAll,
            63,
            ResourceRequest::default(),
        )
        .unwrap();
        assert_eq!(profile.resources().pids_max, 1024);
    }

    #[test]
    fn standard_v1_requires_no_new_privileges() {
        let profile = standard_v1(
            vec![],
            &LinuxCapabilityRequest::default(),
            &LinuxCapabilityGate::AllowAll,
            63,
            ResourceRequest::default(),
        )
        .unwrap();
        assert!(profile.no_new_privileges().required);
    }

    #[test]
    fn standard_v1_denies_device_nodes_and_setid_bits() {
        let profile = standard_v1(
            vec![],
            &LinuxCapabilityRequest::default(),
            &LinuxCapabilityGate::AllowAll,
            63,
            ResourceRequest::default(),
        )
        .unwrap();
        assert!(profile.filesystem().device_nodes_denied);
        assert!(profile.filesystem().setid_bits_denied);
    }

    #[test]
    fn standard_v1_evidence_authority_is_profile_owned_for_every_control() {
        let profile = standard_v1(
            vec![],
            &LinuxCapabilityRequest::default(),
            &LinuxCapabilityGate::AllowAll,
            63,
            ResourceRequest::default(),
        )
        .unwrap();
        for control in IsolationControl::ALL {
            assert_eq!(
                profile.allowed_evidence_authorities().get(&control),
                Some(&BTreeSet::from([EvidenceAuthority::Profile]))
            );
        }
    }

    #[test]
    fn standard_v1_propagates_capability_resolution_errors() {
        let admin = LinuxCapability::parse("CAP_SYS_ADMIN").unwrap();
        let request = LinuxCapabilityRequest {
            add: BTreeSet::from([admin]),
            drop: BTreeSet::new(),
        };
        let err = standard_v1(
            vec![],
            &request,
            &LinuxCapabilityGate::DenyAllAdditions,
            63,
            ResourceRequest::default(),
        )
        .unwrap_err();
        assert_eq!(err, IsolationError::CapabilityAdditionDenied(admin));
    }

    #[test]
    fn compatibility_v1_requires_operator_authorization() {
        // The public builder always takes one, so this exercises the shared
        // validator directly the way trusted deserialization of a
        // hand-crafted payload with no authorization would.
        let fields = ResolvedIsolationProfileFields {
            schema_version: CURRENT_SCHEMA_VERSION,
            profile: IsolationProfileId::CompatibilityV1,
            identity: IdentityPolicy {
                boundary: IdentityBoundary::MappedNonRoot,
                degraded: None,
            },
            process: ProcessPolicy {
                boundary: ProcessBoundary::Required,
                degraded: None,
            },
            mount: MountPolicy {
                host_mounts: vec![],
                degraded: None,
            },
            network: NetworkPolicy {
                boundary: NetworkBoundary::NonHost,
                degraded: None,
            },
            capabilities: ResolvedLinuxCapabilities {
                effective: BTreeSet::new(),
                inheritable: BTreeSet::new(),
                ambient: BTreeSet::new(),
                degraded: None,
            },
            no_new_privileges: NoNewPrivilegesPolicy {
                required: true,
                degraded: None,
            },
            syscall_filter: SyscallFilterPolicy {
                artifact: SyscallFilterArtifact::StandardV1MountImmutability,
                degraded: None,
            },
            filesystem: FilesystemPolicy {
                device_nodes_denied: true,
                setid_bits_denied: true,
                degraded: None,
            },
            resources: ResourcePolicy {
                pids_max: 1024,
                memory_limit_bytes: None,
                cpu_weight: None,
                degraded: None,
            },
            allowed_evidence_authorities: BTreeMap::new(),
            operator_authorization: None,
        };
        let err = ResolvedIsolationProfile::validate_and_build(fields).unwrap_err();
        assert_eq!(err, IsolationError::CompatibilityRequiresAuthorization);
    }

    #[test]
    fn compatibility_v1_rejects_degradation_with_empty_reason() {
        let degradations = BTreeMap::from([(
            IsolationControl::Network,
            Degraded {
                reason: "   ".to_string(),
            },
        )]);
        let err = compatibility_v1(
            OperatorAuthorizationId::new("ticket-123"),
            degradations,
            vec![],
            &LinuxCapabilityRequest::default(),
            &LinuxCapabilityGate::AllowAll,
            63,
            ResourceRequest::default(),
        )
        .unwrap_err();
        assert_eq!(err, IsolationError::DegradationMissingReason);
    }

    #[test]
    fn compatibility_v1_accepts_an_authorized_degradation() {
        let degradations = BTreeMap::from([(
            IsolationControl::Network,
            Degraded {
                reason: "host network required for a legacy integration test".to_string(),
            },
        )]);
        let profile = compatibility_v1(
            OperatorAuthorizationId::new("ticket-123"),
            degradations,
            vec![],
            &LinuxCapabilityRequest::default(),
            &LinuxCapabilityGate::AllowAll,
            63,
            ResourceRequest::default(),
        )
        .unwrap();
        assert!(profile.network().degraded.is_some());
        assert!(profile.identity().degraded.is_none());
        assert_eq!(
            profile
                .operator_authorization()
                .map(OperatorAuthorizationId::as_str),
            Some("ticket-123")
        );
        assert_eq!(
            profile
                .allowed_evidence_authorities()
                .get(&IsolationControl::Network),
            Some(&BTreeSet::from([EvidenceAuthority::Adapter]))
        );
    }

    #[test]
    fn standard_v1_round_trips_through_json() {
        let profile = standard_v1(
            vec![HostMountRule {
                host_path: "/data".to_string(),
                access: MountAccessMode::ReadOnly,
            }],
            &LinuxCapabilityRequest::default(),
            &LinuxCapabilityGate::AllowAll,
            63,
            ResourceRequest::default(),
        )
        .unwrap();
        let json = serde_json::to_string(&profile).unwrap();
        let decoded: ResolvedIsolationProfile = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, profile);
    }

    #[test]
    fn standard_v1_direct_serialization_omits_absent_operator_authorization() {
        // Regression for the direct `Serialize` impl on `ResolvedIsolationProfile`
        // (not the `Fields` mirror type used by `compute_isolation_digest`):
        // without `#[serde(skip_serializing_if = "Option::is_none")]` on
        // `operator_authorization`, this emitted an explicit `"operator_authorization":
        // null` for a `standard-v1` profile, violating the design's canonical-JSON
        // rule that absent optional fields are omitted rather than nulled. The
        // existing round-trip test doesn't catch this because `Option<T>`
        // deserializes `null` as `None` regardless of the serialize-side attribute,
        // so the asymmetry is silent unless the raw JSON shape itself is inspected.
        let profile = standard_v1(
            vec![],
            &LinuxCapabilityRequest::default(),
            &LinuxCapabilityGate::AllowAll,
            63,
            ResourceRequest::default(),
        )
        .unwrap();
        assert!(profile.operator_authorization().is_none());

        let json = serde_json::to_value(&profile).unwrap();
        assert!(
            !json
                .as_object()
                .unwrap()
                .contains_key("operator_authorization"),
            "expected operator_authorization key to be omitted when None, got: {json}"
        );
    }

    #[test]
    fn unknown_schema_version_fails_closed_on_deserialize() {
        let profile = standard_v1(
            vec![],
            &LinuxCapabilityRequest::default(),
            &LinuxCapabilityGate::AllowAll,
            63,
            ResourceRequest::default(),
        )
        .unwrap();
        let mut json: serde_json::Value = serde_json::to_value(&profile).unwrap();
        json["schema_version"] = serde_json::Value::from(99_u32);
        let err = serde_json::from_value::<ResolvedIsolationProfile>(json).unwrap_err();
        assert!(
            err.to_string()
                .contains("unknown isolation profile schema version 99"),
            "got: {err}"
        );
    }
}
