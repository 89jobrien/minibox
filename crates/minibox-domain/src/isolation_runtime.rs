//! The aggregate isolation runtime port, staged-release typestate, and
//! failure/cleanup ownership model.
//!
//! Implements the runtime-port half of
//! `docs/designs/2026-09-21-isolation-admission-lifecycle-design.md` (the
//! policy-resolution half lives in [`crate::isolation`]'s `resolve_isolation_mode`).
//! This module defines *only* the port contract and the values that flow
//! across it — per-platform control application, the daemon's persisted
//! container-state machine, and wire protocol/client behavior are all out of
//! scope here (see the design's "Out Of Scope" section).

use std::collections::BTreeSet;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::isolation::{
    IsolationControl, IsolationDigest, IsolationError, ResolvedIsolationProfile,
};

// ---------------------------------------------------------------------------
// Plan / stage / release request and result values
// ---------------------------------------------------------------------------

/// What the aggregate runtime is asked to plan for one workload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolationPlanRequest {
    /// The resolved profile this workload must run under.
    pub profile: ResolvedIsolationProfile,
    /// The workload's digest, as computed by the caller. Bound into the
    /// returned [`IsolationPlan`] so staging can verify nothing drifted
    /// between planning and staging.
    pub workload_digest: IsolationDigest,
}

/// The result of a successful, non-mutating `plan()` call.
///
/// `plan()` returning `Ok` is a promise, not a mutation: the runtime has
/// confirmed it *can* satisfy every control in the profile, but has not yet
/// allocated or touched any resource.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolationPlan {
    /// Deterministic digest over the plan's content, bound into the
    /// execution manifest and re-checked at every later stage.
    pub plan_digest: IsolationDigest,
    /// The runtime generation (adapter version + environment digest) this
    /// plan was computed against. A plan is only valid against the
    /// generation that produced it.
    pub runtime_generation: IsolationDigest,
}

/// Binds a plan to the container it was planned for, carrying the digests
/// every later stage must match against.
///
/// Mirrors the admission-lifecycle design's `ExecutionManifestBinding`. Kept
/// here rather than in `execution_manifest.rs` because every field is
/// isolation-specific; a future pass may fold this into the execution
/// manifest proper once `minibox/daemon/handler/run.rs` threads both
/// together.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsolationManifestBinding {
    /// The container this binding is for.
    pub container_id: String,
    /// The workload digest this plan was computed against.
    pub workload_digest: IsolationDigest,
    /// The resolved profile's policy digest.
    pub policy_digest: IsolationDigest,
    /// The plan digest returned by `plan()`.
    pub plan_digest: IsolationDigest,
    /// The runtime generation `plan()` was computed against.
    pub runtime_generation: IsolationDigest,
}

/// What `stage()` is asked to allocate for one planned container.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolationStageRequest {
    /// The plan being staged.
    pub plan: IsolationPlan,
    /// The binding tying this stage attempt to its container and digests.
    pub binding: IsolationManifestBinding,
}

/// Where a control's enforcement evidence actually came from, once staged.
///
/// This is the *runtime's* record of what it did, as opposed to
/// [`crate::isolation::EvidenceAuthority`], which is the *profile's* record
/// of what evidence authority is acceptable for a control.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolationEvidenceRecord {
    /// Which control this evidence is for.
    pub control: IsolationControl,
    /// Opaque, adapter-defined evidence payload (e.g. a readback of the
    /// applied kernel state, or a guest-agent attestation blob).
    pub evidence: String,
}

/// The full evidence bundle a staged container carries.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IsolationEvidence {
    /// One record per control the runtime staged.
    pub records: Vec<IsolationEvidenceRecord>,
}

/// Identifies the concrete subject (process, guest session, etc.) a staged
/// allocation produced.
///
/// At least one of `process_id`/`guest_subject_id` is present; which one
/// depends on the adapter (native populates `process_id`, a VM-guest
/// adapter populates `guest_subject_id`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolationSubject {
    /// The subject's host PID, for adapters with direct host process
    /// visibility.
    pub process_id: Option<u32>,
    /// The subject's start-time tick count, used together with `process_id`
    /// to detect PID reuse between staging and release.
    pub process_start_time_ticks: Option<u64>,
    /// An opaque guest-side subject identifier, for adapters whose subject
    /// lives inside a VM/guest session rather than as a host process.
    pub guest_subject_id: Option<String>,
    /// A digest identifying this specific subject's generation, bound into
    /// evidence to prevent replay across restarts of the same adapter.
    pub subject_generation: IsolationDigest,
}

/// An opaque handle identifying one staged-or-later allocation, used to
/// address `wait`/`stop`/`cleanup` calls back to the same allocation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IsolationHandle(String);

impl IsolationHandle {
    /// Wrap an adapter-assigned handle value.
    #[must_use]
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    /// The handle's string form.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A staged, blocked-pending-release allocation.
///
/// Staging has allocated resources and prepared the subject but has not
/// released it to run — release is a separate, deliberate step. A
/// `StagedIsolation` is not yet trusted by [`IsolationRuntime::release`];
/// only a [`ValidatedStagedIsolation`] is.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StagedIsolation {
    /// The handle identifying this allocation for later calls.
    pub handle: IsolationHandle,
    /// The staged subject.
    pub subject: IsolationSubject,
    /// The evidence the runtime collected while staging.
    pub evidence: IsolationEvidence,
}

/// A [`StagedIsolation`] that has passed [`validate_staged_isolation`].
///
/// This is a typestate wrapper: its field is private, so the only way to
/// construct one is to call [`validate_staged_isolation`] with a plan and
/// binding that actually match the staged value. [`IsolationRuntime::release`]
/// takes this type rather than [`StagedIsolation`] precisely so an adapter
/// cannot be handed — and cannot smuggle in — an allocation that was never
/// checked against its plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidatedStagedIsolation(StagedIsolation);

impl ValidatedStagedIsolation {
    /// The validated handle.
    #[must_use]
    pub const fn handle(&self) -> &IsolationHandle {
        &self.0.handle
    }

    /// The validated subject.
    #[must_use]
    pub const fn subject(&self) -> &IsolationSubject {
        &self.0.subject
    }

    /// The validated evidence.
    #[must_use]
    pub const fn evidence(&self) -> &IsolationEvidence {
        &self.0.evidence
    }
}

/// Validate a [`StagedIsolation`] against the plan and binding it was
/// supposedly staged for.
///
/// This is the *only* way to construct a [`ValidatedStagedIsolation`].
/// Checks that the staged evidence covers every control the plan requires,
/// and that the binding's digests are internally consistent (plan digest
/// matches, runtime generation matches).
///
/// # Errors
///
/// Returns [`IsolationValidationFailure`] if any control lacks evidence, or
/// if the plan/binding digests are inconsistent with each other.
pub fn validate_staged_isolation(
    plan: &IsolationPlan,
    binding: &IsolationManifestBinding,
    staged: StagedIsolation,
) -> Result<ValidatedStagedIsolation, Box<IsolationValidationFailure>> {
    if binding.plan_digest != plan.plan_digest {
        return Err(Box::new(IsolationValidationFailure {
            staged,
            reason: "binding's plan_digest does not match the plan being validated".to_string(),
        }));
    }
    if binding.runtime_generation != plan.runtime_generation {
        return Err(Box::new(IsolationValidationFailure {
            staged,
            reason: "binding's runtime_generation does not match the plan's".to_string(),
        }));
    }

    let evidenced: BTreeSet<IsolationControl> = staged
        .evidence
        .records
        .iter()
        .map(|record| record.control)
        .collect();
    let missing = IsolationControl::ALL
        .into_iter()
        .find(|control| !evidenced.contains(control));
    if let Some(control) = missing {
        return Err(Box::new(IsolationValidationFailure {
            staged,
            reason: format!("no evidence recorded for control {control:?}"),
        }));
    }

    Ok(ValidatedStagedIsolation(staged))
}

/// Validation failure from [`validate_staged_isolation`]. Carries the staged
/// value back so the caller can retry cleanup against it.
#[derive(Debug)]
pub struct IsolationValidationFailure {
    /// The staged allocation that failed validation.
    pub staged: StagedIsolation,
    /// Human-readable reason.
    pub reason: String,
}

/// The result of a successful [`IsolationRuntime::release`] call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolatedSpawn {
    /// The handle identifying the running allocation.
    pub handle: IsolationHandle,
    /// The subject, unchanged from staging — release cannot substitute a new
    /// process or evidence object.
    pub subject: IsolationSubject,
    /// The evidence, unchanged from staging.
    pub evidence: IsolationEvidence,
}

/// The exit status of a released, now-exited subject.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IsolationExit {
    /// The process/subject exit code.
    pub code: i32,
}

// ---------------------------------------------------------------------------
// Failure and cleanup ownership
// ---------------------------------------------------------------------------

/// A partially-allocated subject left behind by a failed `stage()` call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PartialIsolationAllocation {
    /// The handle identifying the partial allocation, for retrying cleanup.
    pub handle: IsolationHandle,
    /// Which controls were actually allocated before the failure.
    pub allocated_controls: BTreeSet<IsolationControl>,
}

/// Whether a failed operation left anything behind to clean up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RollbackStatus {
    /// The adapter proved no allocation remains; nothing to retry.
    Complete,
    /// A partial allocation remains; retry idempotent cleanup against it.
    CleanupRequired,
}

/// Failure from [`IsolationRuntime::stage`].
#[derive(Debug)]
pub struct IsolationStageFailure {
    /// The underlying error.
    pub error: IsolationError,
    /// The partial allocation left behind, if any.
    pub partial: Option<PartialIsolationAllocation>,
    /// Whether cleanup is still required.
    pub rollback: RollbackStatus,
}

/// Failure from [`IsolationRuntime::release`]. Carries the handle and
/// subject back so the caller can retry cleanup against the same
/// allocation.
#[derive(Debug)]
pub struct IsolationReleaseFailure {
    /// The underlying error.
    pub error: IsolationError,
    /// The handle of the allocation that failed to release.
    pub handle: IsolationHandle,
    /// The subject that was staged (and never ran).
    pub subject: IsolationSubject,
}

/// Evidence that a cleanup attempt ran, successfully or not.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CleanupEvidence {
    /// Whether every allocated control was confirmed released.
    pub complete: bool,
    /// Human-readable detail, for operator-facing diagnostics.
    pub detail: String,
}

// ---------------------------------------------------------------------------
// The aggregate port
// ---------------------------------------------------------------------------

/// The aggregate isolation runtime port.
///
/// One implementation per adapter (native, colima, gke, krun, smolvm, vz,
/// winbox — see `docs/designs/2026-09-21-isolation-adapter-qualification-design.md`).
/// The implementation owns the filesystem, network, limiter, raw runtime,
/// and rollback ports internally; callers (the daemon's run handler) only
/// ever see this trait, never the mutation ports it composes.
#[async_trait]
pub trait IsolationRuntime: Send + Sync {
    /// Report this runtime's advisory, generation-bound capabilities.
    ///
    /// # Errors
    ///
    /// Returns an error if the capability probe itself fails (e.g. the
    /// adapter cannot determine its own `cap_last_cap`).
    async fn capabilities(&self) -> Result<IsolationRuntimeCapabilities, IsolationError>;

    /// Plan a workload without performing any mutation.
    ///
    /// # Errors
    ///
    /// Returns an error if the runtime cannot satisfy every control in the
    /// requested profile without degradation.
    async fn plan(&self, request: &IsolationPlanRequest) -> Result<IsolationPlan, IsolationError>;

    /// Allocate resources and stage a blocked subject for the planned
    /// workload. Does not release it to run.
    ///
    /// # Errors
    ///
    /// Returns [`IsolationStageFailure`] on any failure. The failure value
    /// always states whether cleanup is still required.
    async fn stage(
        &self,
        request: IsolationStageRequest,
    ) -> Result<StagedIsolation, IsolationStageFailure>;

    /// Release a validated staged subject to begin running.
    ///
    /// # Errors
    ///
    /// Returns [`IsolationReleaseFailure`] on any failure, carrying the
    /// handle and subject back for cleanup.
    async fn release(
        &self,
        staged: ValidatedStagedIsolation,
    ) -> Result<IsolatedSpawn, IsolationReleaseFailure>;

    /// Wait for a released subject to exit.
    ///
    /// # Errors
    ///
    /// Returns an error if the wait itself fails (not if the subject exits
    /// non-zero — that is a normal [`IsolationExit`]).
    async fn wait(&self, handle: &IsolationHandle) -> Result<IsolationExit, IsolationError>;

    /// Request a released subject stop.
    ///
    /// # Errors
    ///
    /// Returns an error if the stop request itself fails.
    async fn stop(&self, handle: &IsolationHandle) -> Result<(), IsolationError>;

    /// Clean up any resources associated with `handle`. Idempotent — safe to
    /// call repeatedly against the same handle after a partial failure.
    ///
    /// # Errors
    ///
    /// Returns an error if cleanup itself cannot be attempted (not if
    /// cleanup completes but finds nothing to do).
    async fn cleanup(&self, handle: &IsolationHandle) -> Result<CleanupEvidence, IsolationError>;
}

/// Where a staged-release guest protocol binds its evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EvidenceAuthorityKind {
    /// Native kernel readback.
    LocalKernel,
    /// Guest-kernel readback bound to a host session.
    GuestAgent,
    /// VM identity, launch, and boundary evidence.
    Hypervisor,
    /// Container-engine or cluster substrate evidence.
    OuterSandbox,
    /// Compatibility degradation only; never used for standard qualification.
    OperatorAuthorizedCompatibility,
}

/// Advisory, generation-bound capabilities reported by one runtime.
///
/// A capabilities report never grants support by itself — [`IsolationRuntime::plan`]
/// remains authoritative for any specific workload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IsolationRuntimeCapabilities {
    /// Adapter identifier (e.g. `"native"`, `"colima"`, `"smolvm"`).
    pub adapter_id: String,
    /// The adapter implementation version.
    pub implementation_version: String,
    /// Digest of the current environment generation (kernel version, VM
    /// image digest, engine version — whatever makes a capability claim
    /// stale when it changes).
    pub environment_generation: IsolationDigest,
    /// Profile IDs this adapter currently qualifies for, if any.
    pub supported_profiles: Vec<crate::isolation::IsolationProfileId>,
    /// Controls this adapter can plan without degradation.
    pub supported_controls: BTreeSet<IsolationControl>,
    /// Evidence authorities this adapter can produce.
    pub accepted_evidence_authorities: BTreeSet<EvidenceAuthorityKind>,
    /// The running kernel's `cap_last_cap`, where applicable (Linux-kernel
    /// adapters only).
    pub cap_last_cap: Option<u16>,
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::isolation::compute_isolation_digest;

    fn digest(seed: &str) -> IsolationDigest {
        compute_isolation_digest(crate::isolation::IsolationDigestDomain::ProfileV1, &seed)
            .expect("digest computation should not fail for a plain string payload")
    }

    fn evidence_for_every_control() -> IsolationEvidence {
        IsolationEvidence {
            records: IsolationControl::ALL
                .into_iter()
                .map(|control| IsolationEvidenceRecord {
                    control,
                    evidence: "ok".to_string(),
                })
                .collect(),
        }
    }

    fn test_subject() -> IsolationSubject {
        IsolationSubject {
            process_id: Some(1234),
            process_start_time_ticks: Some(1),
            guest_subject_id: None,
            subject_generation: digest("subject-1"),
        }
    }

    fn matching_plan_and_binding() -> (IsolationPlan, IsolationManifestBinding) {
        let plan_digest = digest("plan-1");
        let runtime_generation = digest("gen-1");
        let plan = IsolationPlan {
            plan_digest,
            runtime_generation,
        };
        let binding = IsolationManifestBinding {
            container_id: "container-1".to_string(),
            workload_digest: digest("workload-1"),
            policy_digest: digest("policy-1"),
            plan_digest,
            runtime_generation,
        };
        (plan, binding)
    }

    #[test]
    fn validate_staged_isolation_accepts_full_evidence_and_matching_digests() {
        let (plan, binding) = matching_plan_and_binding();
        let staged = StagedIsolation {
            handle: IsolationHandle::new("h-1"),
            subject: test_subject(),
            evidence: evidence_for_every_control(),
        };

        let validated = validate_staged_isolation(&plan, &binding, staged)
            .expect("full evidence and matching digests should validate");
        assert_eq!(validated.handle().as_str(), "h-1");
        assert_eq!(
            validated.evidence().records.len(),
            IsolationControl::ALL.len()
        );
    }

    #[test]
    fn validate_staged_isolation_rejects_mismatched_plan_digest() {
        let (plan, mut binding) = matching_plan_and_binding();
        binding.plan_digest = digest("a-different-plan");
        let staged = StagedIsolation {
            handle: IsolationHandle::new("h-1"),
            subject: test_subject(),
            evidence: evidence_for_every_control(),
        };

        let failure = validate_staged_isolation(&plan, &binding, staged).unwrap_err();
        assert!(failure.reason.contains("plan_digest"));
    }

    #[test]
    fn validate_staged_isolation_rejects_missing_evidence() {
        let (plan, binding) = matching_plan_and_binding();
        let staged = StagedIsolation {
            handle: IsolationHandle::new("h-1"),
            subject: test_subject(),
            evidence: IsolationEvidence::default(),
        };

        let failure = validate_staged_isolation(&plan, &binding, staged).unwrap_err();
        assert!(failure.reason.contains("no evidence recorded"));
    }

    #[test]
    fn validation_failure_returns_the_staged_value_for_retry() {
        let (plan, binding) = matching_plan_and_binding();
        let staged = StagedIsolation {
            handle: IsolationHandle::new("retry-me"),
            subject: test_subject(),
            evidence: IsolationEvidence::default(),
        };

        let failure = validate_staged_isolation(&plan, &binding, staged).unwrap_err();
        assert_eq!(failure.staged.handle.as_str(), "retry-me");
    }
}
