//! Capability evidence recording for backend conformance tests.
//!
//! A proving test calls [`record_proof`] with [`ProofStatus::Failed`] after its skip
//! checks and before its assertions, then with [`ProofStatus::Validated`] once the
//! assertions pass. A panic therefore leaves the record at `failed`.
//!
//! Every `(capability, test)` pair must appear in the calling file's
//! `CAPABILITY_PROOFS`, which keeps the proof map explicit and checked.
//!
//! Records are written only when `MINIBOX_CAPABILITY_EVIDENCE_DIR` is set. Nothing
//! sets it yet, so today this only validates the proof map.

use std::path::PathBuf;

/// Environment variable naming the directory that receives capability records.
pub const EVIDENCE_DIR_ENV: &str = "MINIBOX_CAPABILITY_EVIDENCE_DIR";

/// Outcome written to a capability record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProofStatus {
    Failed,
    Validated,
}

/// Records the outcome of `test` as proof of `capability` for `adapter`.
///
/// # Panics
///
/// Panics if `(capability, test)` is absent from `proofs`, or if the evidence
/// directory cannot be written while `MINIBOX_CAPABILITY_EVIDENCE_DIR` is set.
pub fn record_proof(
    proofs: &[(&str, &str)],
    adapter: &str,
    capability: &str,
    test: &str,
    status: ProofStatus,
) {
    assert!(
        proofs.contains(&(capability, test)),
        "capability {capability:?} is not proven by {test:?} in this file's CAPABILITY_PROOFS"
    );
    let Some(dir) = std::env::var_os(EVIDENCE_DIR_ENV) else {
        return;
    };
    let dir = PathBuf::from(dir);
    std::fs::create_dir_all(&dir).expect("create capability evidence directory");
    let status = match status {
        ProofStatus::Failed => "failed",
        ProofStatus::Validated => "validated",
    };
    let record = serde_json::json!({
        "schema_version": 1,
        "adapter_id": adapter,
        "capability": capability,
        "status": status,
        "proof_test": test,
    });
    std::fs::write(
        dir.join(format!("{adapter}.{capability}.json")),
        serde_json::to_vec_pretty(&record).expect("serialize capability record"),
    )
    .expect("write capability record");
}
