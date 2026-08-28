//! Canonical, typed adapter capability matrix.
#![allow(
    missing_docs,
    reason = "matrix variants are self-described by stable label methods"
)]
//!
//! This module is the single source of truth consumed by the daemon protocol,
//! CLI, conformance checks, and the feature-matrix documentation.

use serde::{Deserialize, Serialize};

/// Adapter backends represented in the capability matrix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[repr(usize)]
pub enum Backend {
    Native,
    Gke,
    Colima,
    Smolvm,
    Krun,
    Vz,
    Winbox,
}

impl Backend {
    /// Every backend, in stable display and wire order.
    pub const ALL: [Self; 7] = [
        Self::Native,
        Self::Gke,
        Self::Colima,
        Self::Smolvm,
        Self::Krun,
        Self::Vz,
        Self::Winbox,
    ];

    /// Stable CLI identifier.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::Gke => "gke",
            Self::Colima => "colima",
            Self::Smolvm => "smolvm",
            Self::Krun => "krun",
            Self::Vz => "vz",
            Self::Winbox => "winbox",
        }
    }
}

/// Feature-matrix section for a capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityGroup {
    ContainerLifecycle,
    ImageManagement,
    Isolation,
    Networking,
    MountsAndPrivileges,
    Security,
    ExecutionIntegrity,
    StatePersistence,
    Observability,
}

impl CapabilityGroup {
    /// Human-readable section label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::ContainerLifecycle => "Container lifecycle",
            Self::ImageManagement => "Image management",
            Self::Isolation => "Isolation",
            Self::Networking => "Networking",
            Self::MountsAndPrivileges => "Mounts & privileges",
            Self::Security => "Security",
            Self::ExecutionIntegrity => "Execution integrity",
            Self::StatePersistence => "State persistence",
            Self::Observability => "Observability",
        }
    }
}

/// A user-visible backend capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Pull,
    Run,
    Stop,
    Remove,
    List,
    PauseResume,
    Restart,
    Exec,
    Logs,
    Events,
    DockerHubV2,
    Ghcr,
    ParallelLayerPull,
    PruneRmi,
    Push,
    Commit,
    Build,
    PidNamespace,
    MountNamespace,
    NetworkNamespace,
    UtsNamespace,
    IpcNamespace,
    CgroupsV2,
    OverlayFs,
    Bridge,
    PortForwarding,
    Dns,
    BindMounts,
    PrivilegedMode,
    PeerCredentials,
    TarPathValidation,
    SetuidStripping,
    DeviceNodeRejection,
    LayerDigestVerification,
    RequestFrameLimits,
    EnvironmentRedaction,
    ExecutionManifest,
    ManifestVerification,
    AdmissionPolicy,
    StatePersistence,
    PidReconciliation,
    StructuredTracing,
    OtlpExport,
}

impl Capability {
    /// Every capability, in stable display and wire order.
    pub const ALL: [Self; 43] = [
        Self::Pull,
        Self::Run,
        Self::Stop,
        Self::Remove,
        Self::List,
        Self::PauseResume,
        Self::Restart,
        Self::Exec,
        Self::Logs,
        Self::Events,
        Self::DockerHubV2,
        Self::Ghcr,
        Self::ParallelLayerPull,
        Self::PruneRmi,
        Self::Push,
        Self::Commit,
        Self::Build,
        Self::PidNamespace,
        Self::MountNamespace,
        Self::NetworkNamespace,
        Self::UtsNamespace,
        Self::IpcNamespace,
        Self::CgroupsV2,
        Self::OverlayFs,
        Self::Bridge,
        Self::PortForwarding,
        Self::Dns,
        Self::BindMounts,
        Self::PrivilegedMode,
        Self::PeerCredentials,
        Self::TarPathValidation,
        Self::SetuidStripping,
        Self::DeviceNodeRejection,
        Self::LayerDigestVerification,
        Self::RequestFrameLimits,
        Self::EnvironmentRedaction,
        Self::ExecutionManifest,
        Self::ManifestVerification,
        Self::AdmissionPolicy,
        Self::StatePersistence,
        Self::PidReconciliation,
        Self::StructuredTracing,
        Self::OtlpExport,
    ];

    /// Human-readable row label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Pull => "pull",
            Self::Run => "run",
            Self::Stop => "stop",
            Self::Remove => "rm",
            Self::List => "ps",
            Self::PauseResume => "pause/resume",
            Self::Restart => "restart",
            Self::Exec => "exec (-it)",
            Self::Logs => "logs",
            Self::Events => "events",
            Self::DockerHubV2 => "Docker Hub v2",
            Self::Ghcr => "ghcr.io",
            Self::ParallelLayerPull => "Parallel layer pull",
            Self::PruneRmi => "prune / rmi",
            Self::Push => "push (exp)",
            Self::Commit => "commit (exp)",
            Self::Build => "build (exp)",
            Self::PidNamespace => "PID namespace",
            Self::MountNamespace => "Mount namespace",
            Self::NetworkNamespace => "Network namespace",
            Self::UtsNamespace => "UTS namespace",
            Self::IpcNamespace => "IPC namespace",
            Self::CgroupsV2 => "cgroups v2",
            Self::OverlayFs => "Overlay FS",
            Self::Bridge => "Bridge (exp)",
            Self::PortForwarding => "Port forwarding",
            Self::Dns => "DNS",
            Self::BindMounts => "Bind mounts (-v)",
            Self::PrivilegedMode => "Privileged mode",
            Self::PeerCredentials => "SO_PEERCRED auth",
            Self::TarPathValidation => "Tar path validation",
            Self::SetuidStripping => "Setuid stripping",
            Self::DeviceNodeRejection => "Device node rejection",
            Self::LayerDigestVerification => "Layer digest verify",
            Self::RequestFrameLimits => "Request frame limits",
            Self::EnvironmentRedaction => "Env redaction in logs",
            Self::ExecutionManifest => "Execution manifest",
            Self::ManifestVerification => "manifest get/verify",
            Self::AdmissionPolicy => "Admission policy gate",
            Self::StatePersistence => "Records survive restart",
            Self::PidReconciliation => "PID reconciliation",
            Self::StructuredTracing => "Structured tracing",
            Self::OtlpExport => "OTLP export (opt-in)",
        }
    }
}

/// Infrastructure that provides a capability on behalf of minibox.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityProvider {
    LimaVm,
    Vm,
    CopyFilesystem,
    Nerdctl,
}

/// Support level for one capability on one backend.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "status", content = "provider", rename_all = "snake_case")]
pub enum CapabilitySupport {
    Supported,
    Unsupported,
    Limited,
    ProvidedBy(CapabilityProvider),
}

impl CapabilitySupport {
    /// Compact value used by the human-readable CLI table.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Supported => "Yes",
            Self::Unsupported => "No",
            Self::Limited => "Limited",
            Self::ProvidedBy(CapabilityProvider::LimaVm) => "Lima VM",
            Self::ProvidedBy(CapabilityProvider::Vm) => "VM",
            Self::ProvidedBy(CapabilityProvider::CopyFilesystem) => "Copy",
            Self::ProvidedBy(CapabilityProvider::Nerdctl) => "nerdctl",
        }
    }
}

/// Capability support across every backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRow {
    pub group: CapabilityGroup,
    pub capability: Capability,
    pub support: [CapabilitySupport; 7],
}

impl CapabilityRow {
    /// Support level for `backend`.
    #[must_use]
    pub const fn for_backend(&self, backend: Backend) -> CapabilitySupport {
        self.support[backend as usize]
    }
}

/// Versioned matrix returned over the daemon protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityMatrix {
    pub schema_version: u16,
    pub backends: Vec<Backend>,
    pub capabilities: Vec<CapabilityRow>,
}

impl CapabilityMatrix {
    /// Query support without parsing documentation or CLI text.
    #[must_use]
    pub fn support(&self, backend: Backend, capability: Capability) -> Option<CapabilitySupport> {
        self.capabilities
            .iter()
            .find(|row| row.capability == capability)
            .map(|row| row.for_backend(backend))
    }
}

use CapabilityProvider::{CopyFilesystem as CopyFs, LimaVm, Nerdctl, Vm};
use CapabilitySupport::{Limited as L, ProvidedBy as P, Supported as Y, Unsupported as N};

macro_rules! row {
    ($group:ident, $capability:ident, $support:expr) => {
        CapabilityRow {
            group: CapabilityGroup::$group,
            capability: Capability::$capability,
            support: $support,
        }
    };
}

/// Build the canonical capability matrix.
#[must_use]
pub fn capability_matrix() -> CapabilityMatrix {
    let all_yes = [Y, Y, Y, Y, Y, Y, Y];
    let runtime_yes = [Y, Y, Y, Y, Y, Y, N];
    let lifecycle = [Y, Y, Y, Y, Y, Y, N];
    let vm_isolation = [Y, N, P(LimaVm), P(Vm), P(Vm), P(Vm), N];
    CapabilityMatrix {
        schema_version: 1,
        backends: Backend::ALL.to_vec(),
        capabilities: vec![
            row!(ContainerLifecycle, Pull, lifecycle),
            row!(ContainerLifecycle, Run, lifecycle),
            row!(ContainerLifecycle, Stop, lifecycle),
            row!(ContainerLifecycle, Remove, lifecycle),
            row!(ContainerLifecycle, List, lifecycle),
            row!(ContainerLifecycle, PauseResume, [Y, N, N, N, N, N, N]),
            row!(ContainerLifecycle, Restart, lifecycle),
            row!(ContainerLifecycle, Exec, [Y, N, L, N, N, N, N]),
            row!(ContainerLifecycle, Logs, [Y, N, L, N, N, N, N]),
            row!(ContainerLifecycle, Events, [Y, Y, N, N, N, N, N]),
            row!(ImageManagement, DockerHubV2, runtime_yes),
            row!(ImageManagement, Ghcr, runtime_yes),
            row!(ImageManagement, ParallelLayerPull, runtime_yes),
            row!(ImageManagement, PruneRmi, [Y, N, N, N, N, N, N]),
            row!(ImageManagement, Push, [Y, Y, Y, N, N, N, N]),
            row!(ImageManagement, Commit, [Y, N, Y, N, N, N, N]),
            row!(ImageManagement, Build, [Y, N, Y, Y, N, N, N]),
            row!(Isolation, PidNamespace, vm_isolation),
            row!(Isolation, MountNamespace, vm_isolation),
            row!(Isolation, NetworkNamespace, vm_isolation),
            row!(Isolation, UtsNamespace, vm_isolation),
            row!(Isolation, IpcNamespace, vm_isolation),
            row!(Isolation, CgroupsV2, [Y, N, P(LimaVm), P(Vm), N, Y, N]),
            row!(Isolation, OverlayFs, [Y, P(CopyFs), P(Nerdctl), N, N, Y, N]),
            row!(Networking, Bridge, [Y, N, N, N, N, N, N]),
            row!(Networking, PortForwarding, [N, N, N, N, N, N, N]),
            row!(Networking, Dns, [N, N, N, N, N, N, N]),
            row!(MountsAndPrivileges, BindMounts, [Y, N, N, N, N, N, N]),
            row!(MountsAndPrivileges, PrivilegedMode, [Y, N, N, N, N, N, N]),
            row!(Security, PeerCredentials, runtime_yes),
            row!(Security, TarPathValidation, all_yes),
            row!(Security, SetuidStripping, all_yes),
            row!(Security, DeviceNodeRejection, all_yes),
            row!(Security, LayerDigestVerification, runtime_yes),
            row!(Security, RequestFrameLimits, runtime_yes),
            row!(Security, EnvironmentRedaction, runtime_yes),
            row!(ExecutionIntegrity, ExecutionManifest, runtime_yes),
            row!(ExecutionIntegrity, ManifestVerification, runtime_yes),
            row!(ExecutionIntegrity, AdmissionPolicy, runtime_yes),
            row!(StatePersistence, StatePersistence, runtime_yes),
            row!(StatePersistence, PidReconciliation, [Y, N, N, N, N, N, N]),
            row!(Observability, StructuredTracing, runtime_yes),
            row!(Observability, OtlpExport, runtime_yes),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matrix_contains_every_backend_and_capability_once() {
        let matrix = capability_matrix();
        assert_eq!(matrix.backends, Backend::ALL);
        assert_eq!(matrix.capabilities.len(), Capability::ALL.len());
        for capability in Capability::ALL {
            assert_eq!(
                matrix
                    .capabilities
                    .iter()
                    .filter(|row| row.capability == capability)
                    .count(),
                1
            );
        }
    }

    #[test]
    fn matrix_exposes_backend_specific_support() {
        let matrix = capability_matrix();
        assert_eq!(
            matrix.support(Backend::Native, Capability::PauseResume),
            Some(CapabilitySupport::Supported)
        );
        assert_eq!(
            matrix.support(Backend::Colima, Capability::Exec),
            Some(CapabilitySupport::Limited)
        );
        assert_eq!(
            matrix.support(Backend::Smolvm, Capability::PidNamespace),
            Some(CapabilitySupport::ProvidedBy(CapabilityProvider::Vm))
        );
    }

    #[test]
    fn matrix_roundtrips_json() {
        let matrix = capability_matrix();
        let json = serde_json::to_string(&matrix).expect("serialize matrix");
        let decoded: CapabilityMatrix = serde_json::from_str(&json).expect("deserialize matrix");
        assert_eq!(decoded, matrix);
    }
}
