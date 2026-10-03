# Plan: signed-governed-execution-daemon

## Goal

Implement the signed governed execution path described in `docs/designs/2026-10-02-signed-governed-execution-daemon-design.md` — externally signed envelope admission with Ed25519 verification, an atomic transactional ledger, and a staged native run engine — while leaving ordinary `Run` behavior provably unchanged.

## Architecture

- **Crates affected**: `minibox-domain` (new `governed_envelope`, `trust_store`, `governed_ledger`, `governed_authorization` modules; two defaulted methods on `ContainerRuntime`), `minibox-core` (additive `GovernedRun` protocol variants), `minibox` (verifier, ledger, trust-store loader, admission state machine, extracted `RunEngine`, native adapter override), `miniboxd` (`GovernedExecutorConfig`, native-only gating), `minibox-testsuite` (staged-spawn conformance), `xtask` (smolvm provisioning gate and VM harness fixes).
- **New traits/types**:
  - `minibox-domain::governed_authorization::AuthorizationVerifier` — `verify`, `identifier`
  - `minibox-domain::governed_ledger::GovernedLedger` — `reserve`, `append`, `load`, `list_interrupted`
  - `minibox-domain::governed_envelope::{GovernedEnvelope, Grant, AuthorityScope, GovernedRequest, GovernedMount, ResourceLimits, GrantedCapabilities, Signature, VerifyingKey, CanonicalizationError}`
  - `minibox-domain::governed_envelope::{canonical_signing_bytes, validate_structure, narrow_effective}`
  - `minibox-domain::trust_store::{TrustStoreEntry, TrustStoreSnapshot, TrustStoreError}`
  - `minibox-domain::governed_ledger::{ExecutionPhase, LedgerRecord, ReservationClaim, OwnershipSelector, ResourceLocator, ResourceKind, HeadDigest, LedgerError}`
  - `minibox-domain::runtime::{ResourceIdentity, StagedSpawnResult, StagedError}`
  - `minibox::daemon::governed::store::FileGovernedLedger`
  - `minibox::daemon::governed::authorization::Ed25519AuthorizationVerifier`
  - `minibox::daemon::governed::trust_store::load_trust_store`
- **Data flow**: raw client frame → frame validation (1 MiB cap, duplicate-key rejection) → structural parse → `canonical_signing_bytes` → `Ed25519AuthorizationVerifier::verify` (strict) → trust resolution → temporal and audience checks → scope and policy-digest checks → local `PolicyEvaluator` deny-only ceiling → `narrow_effective` → `GovernedLedger::reserve` → `RunEngine` stages → daemon journals PID and `Started` then calls `ContainerRuntime::release` → wait, discover, finalize by CAS → idempotent cleanup → terminal envelope on the response stream.

### Deviations from the approved design document

Two, both deliberate and flagged here rather than applied silently:

1. **`CapabilitySet` is renamed `GrantedCapabilities`.** The existing `minibox_domain::BackendCapabilitySet` (`crates/minibox-domain/src/capability.rs:109`) names a different axis — what a backend _supports_. Keeping `CapabilitySet` invites confusion between "the backend can do this" and "the signature authorizes this". The concepts are orthogonal; only the name collided. The design doc should be updated to match.
2. **`GovernedLedger` methods are `async`.** Every port in this workspace that performs I/O or is held behind `Arc` in the daemon is async (`ContainerRuntime::spawn_process` at `crates/minibox-domain/src/runtime.rs:206`). The design doc showed them sync; making them async is required for the ledger to read and `fsync` files without blocking the runtime.

## Tech Stack

- **Rust edition**: 2024. MSRV **1.85** (`Cargo.toml:30`).
- **Workspace lints** (`Cargo.toml:112-124`): `unwrap_used`, `expect_used`, `panic`, `todo`, `unimplemented` all `deny`; `missing_docs` `deny`; pedantic and nursery at `warn`. Tests are exempt — `crates/minibox-domain/src/lib.rs:1-13` applies `#![cfg_attr(test, allow(...))]` for `unwrap_used`, `expect_used`, `panic`, and five others. **Every new public item needs a doc comment.**
- **New dependencies**:
  - `ed25519-dalek` **3.x** — MSRV 1.85, exactly matching this workspace. 2.x needs only 1.60. If 3.x is still pre-release when Task 5 runs, pin 2.x and confirm `verify_strict` exists on it.
  - No other new dependencies. JCS is implemented in-tree against vendored RFC 8785 vectors.
- **Forbidden for the verifier**: the `legacy_compatibility` feature. It reinstates signature malleability. `VerifyingKey::verify_strict` is mandatory; plain `verify()` permits weak low-order keys that sign almost any message.

## Tasks

### Task 1: Scaffold the `governed_envelope` module with request types

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/governed_envelope.rs`, `crates/minibox-domain/src/lib.rs`
**Run**: `cargo nextest run -p minibox-domain governed_envelope`

1. Write the failing test at the bottom of the new file:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> GovernedRequest {
        GovernedRequest {
            image_ref: "alpine:3.20".to_string(),
            image_manifest_digest: "sha256:abc".to_string(),
            command: vec!["/bin/sh".to_string()],
            entrypoint: None,
            network_mode: NetworkMode::None,
            privileged: false,
            mounts: vec![],
            resource_limits: ResourceLimits {
                memory_limit_bytes: Some(256 * 1024 * 1024),
                cpu_weight: None,
                pids_limit: Some(512),
            },
            platform: None,
        }
    }

    #[test]
    fn governed_request_roundtrips_through_json() {
        let json = serde_json::to_string(&request()).expect("serialise");
        let back: GovernedRequest = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.image_ref, "alpine:3.20");
        assert_eq!(back.resource_limits.pids_limit, Some(512));
    }
}
```

2. Confirm FAIL: `cargo nextest run -p minibox-domain governed_envelope` → compile error, `cannot find type GovernedRequest`.

3. Implement the types with docs on every public item:

```rust
//! Signed envelope schema for governed container execution.

use serde::{Deserialize, Serialize};

use crate::networking::NetworkMode;

/// Canonicalization scheme version implemented by this module.
pub const GOVERNED_ENVELOPE_SCHEMA_VERSION: u32 = 1;

/// Domain-separation context prefixed to all signed bytes.
pub const GOVERNED_SIGNING_CONTEXT: &str = "minibox.governed.envelope.v1";

/// What a signed envelope asks to run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedRequest {
    /// Image reference the workload resolves.
    pub image_ref: String,
    /// Expected OCI manifest digest; the image must already be cached and match.
    pub image_manifest_digest: String,
    /// Command and arguments.
    pub command: Vec<String>,
    /// Optional entrypoint override.
    pub entrypoint: Option<Vec<String>>,
    /// Requested network mode.
    pub network_mode: NetworkMode,
    /// Whether privileged mode is requested.
    pub privileged: bool,
    /// Bind mounts applied inside the container.
    pub mounts: Vec<GovernedMount>,
    /// Resource ceilings.
    pub resource_limits: ResourceLimits,
    /// Optional target platform.
    pub platform: Option<String>,
}

/// A bind mount a governed workload requests.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedMount {
    /// Absolute host path.
    pub host_path: String,
    /// Absolute path inside the container.
    pub container_path: String,
    /// Whether the mount is read-only.
    pub read_only: bool,
}

/// Resource ceilings for a governed workload.
///
/// `memory_limit_bytes` and `pids_limit` are bounded to 2^53 - 1 so they
/// survive JCS IEEE 754 double serialization without loss.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ResourceLimits {
    /// Memory ceiling in bytes, at most 2^53 - 1.
    pub memory_limit_bytes: Option<u64>,
    /// CPU weight.
    pub cpu_weight: Option<u32>,
    /// PID ceiling, at most 2^53 - 1.
    pub pids_limit: Option<u64>,
}

/// Capabilities a signature may authorize. Deny-by-default: a capability not
/// present is never available to the workload.
///
/// Distinct from [`crate::capability::BackendCapabilitySet`], which describes
/// what a backend adapter supports rather than what a signature authorizes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantedCapabilities {
    /// Custom environment variables may be supplied.
    pub allow_custom_environment: bool,
    /// Secret material may be injected.
    pub allow_secrets: bool,
    /// Network access is permitted.
    pub allow_network: bool,
    /// Privileged mode is permitted.
    pub allow_privilege: bool,
}
```

Register in `crates/minibox-domain/src/lib.rs` — `pub mod governed_envelope;` after `pub mod execution_policy;`, and `pub use governed_envelope::*;` after `pub use execution_policy::*;`. Both lists are alphabetical; preserve that.

4. Confirm GREEN: `cargo nextest run -p minibox-domain` → all green. Then `cargo clippy -p minibox-domain -- -D warnings` → zero warnings.

5. `git branch --show-current` → verify `develop`. Stop if not.
   Commit: `git commit -m "feat(minibox-domain): add governed request and grant value types"`

### Task 2: Add the envelope, grant, and authority scope types

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/governed_envelope.rs`
**Run**: `cargo nextest run -p minibox-domain governed_envelope`

1. Write the failing test, appended to the existing `mod tests`:

```rust
    #[test]
    fn envelope_roundtrips_with_signature_excluded_from_canonical_bytes() {
        let env = GovernedEnvelope {
            schema_version: GOVERNED_ENVELOPE_SCHEMA_VERSION,
            key_id: "release-signer".to_string(),
            issued_at: SystemTime::UNIX_EPOCH,
            not_before: SystemTime::UNIX_EPOCH,
            not_after: SystemTime::UNIX_EPOCH,
            nonce: [7u8; 32],
            execution_id: "exec-001".to_string(),
            grant: Grant {
                audience: "miniboxd".to_string(),
                subject: "agent-a".to_string(),
                scope: AuthorityScope {
                    permitted_intents: vec!["run".to_string()],
                    permitted_images: vec!["alpine*".to_string()],
                },
                policy_digest: [9u8; 32],
                resource_ceiling: ResourceLimits {
                    memory_limit_bytes: Some(1024 * 1024),
                    cpu_weight: None,
                    pids_limit: Some(64),
                },
                capabilities: GrantedCapabilities::default(),
            },
            request: request(),
            signature: Signature([3u8; 64]),
        };
        let json = serde_json::to_string(&env).expect("serialise");
        let back: GovernedEnvelope = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back, env);
    }
```

2. Confirm FAIL → `cannot find type GovernedEnvelope`.

3. Implement:

```rust
/// The signed unit of authorization.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GovernedEnvelope {
    /// Canonicalization scheme version.
    pub schema_version: u32,
    /// Identifier of the signing key in the trust store.
    pub key_id: String,
    /// When the authority issued the envelope. RFC 3339 UTC, millisecond precision.
    pub issued_at: SystemTime,
    /// Earliest validity instant. RFC 3339 UTC, millisecond precision.
    pub not_before: SystemTime,
    /// Latest validity instant. RFC 3339 UTC, millisecond precision.
    pub not_after: SystemTime,
    /// Single-use value; consumed by the ledger at reservation.
    pub nonce: [u8; 32],
    /// Caller-chosen idempotency key.
    pub execution_id: String,
    /// The authority ceiling.
    pub grant: Grant,
    /// What the envelope asks to run.
    pub request: GovernedRequest,
    /// Ed25519 signature over the canonical bytes. Never part of those bytes.
    pub signature: Signature,
}

/// The ceiling an authority signs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Grant {
    /// Daemon identity this grant is bound to.
    pub audience: String,
    /// Caller identity.
    pub subject: String,
    /// What the key is permitted to authorize.
    pub scope: AuthorityScope,
    /// Digest of the policy the authority approved against.
    pub policy_digest: [u8; 32],
    /// Highest resource limits the authority will permit.
    pub resource_ceiling: ResourceLimits,
    /// Capabilities the authority will permit.
    pub capabilities: GrantedCapabilities,
}

/// The set of things a trusted key may authorize.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorityScope {
    /// Allowed intent verbs.
    pub permitted_intents: Vec<String>,
    /// Glob patterns for permitted images.
    pub permitted_images: Vec<String>,
}

/// An Ed25519 signature. Opaque on purpose: the wire format fixes the
/// algorithm, so this is not abstracted behind a trait.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Signature(pub [u8; 64]);

/// An Ed25519 public key.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct VerifyingKey(pub [u8; 32]);
```

Add `use std::time::SystemTime;` to the imports.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox-domain): add governed envelope, grant, and authority scope"`

### Task 3: Add canonicalization error variants and integer bounds

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/governed_envelope.rs`
**Run**: `cargo nextest run -p minibox-domain governed_envelope`

1. Write the failing tests:

```rust
    #[test]
    fn resource_limits_above_two_pow_53_are_rejected() {
        let too_large = ResourceLimits {
            memory_limit_bytes: Some(1u64 << 53),
            cpu_weight: None,
            pids_limit: None,
        };
        assert_eq!(
            too_large.check_bounds(),
            Err(CanonicalizationError::IntegerOutOfRange)
        );
    }

    #[test]
    fn resource_limits_at_two_pow_53_minus_one_are_accepted() {
        let ok = ResourceLimits {
            memory_limit_bytes: Some((1u64 << 53) - 1),
            cpu_weight: None,
            pids_limit: Some((1u64 << 53) - 1),
        };
        assert_eq!(ok.check_bounds(), Ok(()));
    }
```

2. Confirm FAIL → `no method named check_bounds`.

3. Implement. Add the error enum and the bounds check:

```rust
/// Largest integer that survives JCS IEEE 754 double serialization.
pub const MAX_EXACT_JSON_INTEGER: u64 = (1u64 << 53) - 1;

/// Ways canonicalization can refuse input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CanonicalizationError {
    /// `schema_version` is not `GOVERNED_ENVELOPE_SCHEMA_VERSION`.
    UnsupportedVersion,
    /// A number was NaN or Infinity.
    NonFiniteNumber,
    /// A string was not valid Unicode.
    InvalidUtf8,
    /// An object carried a duplicate property name, which I-JSON forbids.
    DuplicateProperty,
    /// An integer exceeded [`MAX_EXACT_JSON_INTEGER`].
    IntegerOutOfRange,
    /// A timestamp carried sub-millisecond precision that would not survive
    /// the signature boundary.
    TimestampPrecision,
}
```

and on `ResourceLimits`:

```rust
impl ResourceLimits {
    /// Reject values that cannot round-trip through JCS.
    ///
    /// # Errors
    /// Returns [`CanonicalizationError::IntegerOutOfRange`] if either field
    /// exceeds [`MAX_EXACT_JSON_INTEGER`].
    pub const fn check_bounds(&self) -> Result<(), CanonicalizationError> {
        if let Some(v) = self.memory_limit_bytes
            && v > MAX_EXACT_JSON_INTEGER
        {
            return Err(CanonicalizationError::IntegerOutOfRange);
        }
        if let Some(v) = self.pids_limit
            && v > MAX_EXACT_JSON_INTEGER
        {
            return Err(CanonicalizationError::IntegerOutOfRange);
        }
        Ok(())
    }
}
```

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox-domain): bound governed integers to the JCS exact range"`

### Task 4: Implement RFC 8785 canonical signing bytes

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/governed_envelope.rs`, `crates/minibox-domain/tests/rfc8785_vectors.rs`
**Run**: `cargo nextest run -p minibox-domain rfc8785`

1. Write the failing integration test. Vendor the **§3.2.2** sample object verbatim, plus a separate Appendix B number case:

```rust
//! Golden vectors from RFC 8785 (JSON Canonicalization Scheme).

/// Section 3.2.2 sample object, verbatim. Exercises recursive property
/// sorting, ECMAScript string escaping, and array order preservation.
const RFC8785_S_3_2_2_SAMPLE: &str = r#"{"numbers": [333333333.33333329, 1E30, 4.50, 2e-3, 0.000000000000000000000000001], "string": "\u20ac$\u000F\u000aA'\u0042\u0022\u005c\\\"\/", "literals": [null, true, false]}"#;

/// Appendix B table row: IEEE 754 `4340000000000000` -> `9007199254740992`
/// ("Max pos int"). Note (1) recommends integers stay within
/// -9007199254740991..=9007199254740991, which is exactly why
/// `MAX_EXACT_JSON_INTEGER` is 2^53 - 1 rather than 2^53.
const RFC8785_APPENDIX_B_MAX_INT: &str = "9007199254740992";

#[test]
fn canonical_form_sorts_properties_recursively() {
    let value: serde_json::Value =
        serde_json::from_str(RFC8785_S_3_2_2_SAMPLE).expect("parse RFC 8785 sample");
    let bytes = minibox_domain::governed_envelope::canonicalize_json(&value)
        .expect("canonicalize");
    // "literals" < "numbers" < "string" by UTF-16 code unit order.
    assert!(bytes.starts_with(br#"{"literals":[null,true,false],"numbers":"#));
}

#[test]
fn array_order_is_preserved() {
    // The numbers array must keep source order; only object keys are sorted.
    let value: serde_json::Value =
        serde_json::from_str(RFC8785_S_3_2_2_SAMPLE).expect("parse RFC 8785 sample");
    let bytes = minibox_domain::governed_envelope::canonicalize_json(&value)
        .expect("canonicalize");
    let text = String::from_utf8(bytes).expect("utf8");
    assert!(text.contains("[333333333.33333329,1e+30,4.5,0.002,1e-27]"), "got: {text}");
}

#[test]
fn appendix_b_number_samples_round_trip() {
    let value: serde_json::Value =
        serde_json::from_str(RFC8785_APPENDIX_B_MAX_INT).expect("parse");
    let bytes = minibox_domain::governed_envelope::canonicalize_json(&value)
        .expect("canonicalize");
    assert_eq!(bytes, b"9007199254740992");
}
```

2. Confirm FAIL → `cannot find function canonicalize_json`.

3. Implement `canonicalize_json` in the module, following RFC 8785: no whitespace (§3.2.1); object properties sorted by **UTF-16 code unit** value comparing code units as unsigned integers (§3.2.3); array element order unchanged; strings serialized per ECMAScript with `\b \t \n \f \r` and lowercase `\uhhhh` for other control characters (§3.2.2.2); numbers per ECMAScript `Number::toString` (§3.2.2.3, per the Appendix B table); NaN and Infinity return `NonFiniteNumber`; result encoded UTF-8 (§3.2.4).

For property sorting, convert each key to `Vec<u16>` with `.encode_utf16()` and compare those vectors — do not sort `String`s, whose `Ord` is UTF-8 byte order and differs from UTF-16 code unit order for characters above the BMP.

```rust
/// Canonicalize an arbitrary JSON value per RFC 8785.
///
/// # Errors
/// Returns [`CanonicalizationError::NonFiniteNumber`] for NaN or Infinity,
/// [`CanonicalizationError::InvalidUtf8`] for lone surrogates, and
/// [`CanonicalizationError::DuplicateProperty`] for duplicate object keys.
pub fn canonicalize_json(value: &serde_json::Value) -> Result<Vec<u8>, CanonicalizationError>
```

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox-domain): canonicalize JSON per RFC 8785"`

### Task 5: Add `ed25519-dalek` and strict signature verification

**Crate**: `minibox-domain`
**File(s)**: `Cargo.toml`, `crates/minibox-domain/Cargo.toml`, `crates/minibox-domain/src/governed_envelope.rs`
**Run**: `cargo nextest run -p minibox-domain governed_envelope`

1. Add to `[workspace.dependencies]`: `ed25519-dalek = { version = "3", default-features = false, features = ["std"] }`. Add to `crates/minibox-domain/Cargo.toml` `[dependencies]`: `ed25519-dalek = { workspace = true }`.

2. Write the failing test:

```rust
    #[test]
    fn strict_verification_rejects_a_tampered_payload() {
        let sk = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let vk = sk.verifying_key();
        let payload = b"minibox.governed.envelope.v1{}".to_vec();
        let sig = ed25519_dalek::Signer::sign(&sk, &payload);
        let ours = Signature(sig.to_bytes());
        let theirs = VerifyingKey(vk.to_bytes());
        assert!(
            ours.verify_strict(&theirs, &payload).is_ok(),
            "unmodified payload must verify"
        );
        assert_eq!(
            ours.verify_strict(&theirs, b"tampered"),
            Err(CanonicalizationError::InvalidUtf8)
        );
    }
```

Adjust the second assertion's expected variant to whichever `verify_strict` wrapper error you settle on — use a dedicated `SignatureVerification` error rather than overloading `CanonicalizationError`.

3. Confirm FAIL → `cannot find function verify_strict`.

4. Implement on `Signature`:

```rust
impl Signature {
    /// Verify against `key` using strict Ed25519.
    ///
    /// Strict verification is mandatory: plain `verify()` accepts weak
    /// low-order public keys that validate a signature over almost any
    /// message.
    ///
    /// # Errors
    /// Returns an error if the signature does not verify or the key is weak.
    pub fn verify_strict(
        &self,
        key: &VerifyingKey,
        message: &[u8],
    ) -> Result<(), SignatureError> {
        let sig = ed25519_dalek::Signature::from_bytes(&self.0);
        let vk = ed25519_dalek::VerifyingKey::from_bytes(&key.0)
            .map_err(|_| SignatureError::MalformedKey)?;
        vk.verify_strict(message, &sig).map_err(SignatureError::Invalid)
    }
}
```

Add `SignatureError { Invalid, MalformedKey }` with `thiserror::Derive` — `thiserror` is already a domain dependency (`crates/minibox-domain/Cargo.toml`).

5. Confirm GREEN, clippy zero warnings.

6. Commit: `git commit -m "feat(minibox-domain): verify governed signatures with strict Ed25519"`

### Task 6: Implement `canonical_signing_bytes` with domain separation

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/governed_envelope.rs`
**Run**: `cargo nextest run -p minibox-domain governed_envelope`

1. Write the failing test:

```rust
    #[test]
    fn signing_bytes_are_prefixed_and_exclude_the_signature() {
        let mut env = sample_envelope();
        let a = canonical_signing_bytes(&env).expect("canonicalize");
        env.signature = Signature([0xAB; 64]);
        let b = canonical_signing_bytes(&env).expect("canonicalize");
        assert_eq!(a, b, "signature field must not affect signed bytes");
        assert!(a.starts_with(GOVERNED_SIGNING_CONTEXT.as_bytes()));
    }
```

2. Confirm FAIL → `cannot find function canonical_signing_bytes`.

3. Implement:

```rust
/// Produce the exact bytes covered by the envelope signature.
///
/// Serializes every field except `signature`, canonicalizes per RFC 8785,
/// and prefixes [`GOVERNED_SIGNING_CONTEXT`] so a signature over an envelope
/// cannot be replayed over any other structure the daemon accepts.
///
/// # Errors
/// Propagates every [`CanonicalizationError`], plus
/// [`CanonicalizationError::IntegerOutOfRange`] when a resource ceiling
/// exceeds [`MAX_EXACT_JSON_INTEGER`].
pub fn canonical_signing_bytes(
    envelope: &GovernedEnvelope,
) -> Result<Vec<u8>, CanonicalizationError> {
    envelope.grant.resource_ceiling.check_bounds()?;
    envelope.request.resource_limits.check_bounds()?;
    let mut value = serde_json::to_value(envelope).map_err(|_| CanonicalizationError::InvalidUtf8)?;
    if let Some(obj) = value.as_object_mut() {
        obj.remove("signature");
    }
    let mut out = Vec::from(GOVERNED_SIGNING_CONTEXT.as_bytes());
    out.extend_from_slice(&canonicalize_json(&value)?);
    Ok(out)
}
```

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox-domain): domain-separate governed signing bytes"`

### Task 7: Implement `narrow_effective` as a pointwise minimum

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/governed_envelope.rs`
**Run**: `cargo nextest run -p minibox-domain governed_envelope`

1. Write the failing tests:

```rust
    #[test]
    fn narrowing_takes_the_lower_memory_ceiling() {
        let request = request_at(512);
        let grant = grant_at(256);
        let ceiling = request_at(1024);
        let effective = narrow_effective(&request, &grant, &ceiling);
        assert_eq!(effective.resource_limits.memory_limit_bytes, Some(256));
    }

    #[test]
    fn narrowing_can_only_lower_never_raise() {
        let request = request_at(256);
        let grant = grant_at(1024);
        let ceiling = request_at(512);
        let effective = narrow_effective(&request, &grant, &ceiling);
        assert_eq!(effective.resource_limits.memory_limit_bytes, Some(256));
    }

    #[test]
    fn narrowing_cannot_grant_privilege_the_request_never_asked_for() {
        let request = request_at(256); // privileged: false
        let grant = Grant { capabilities: GrantedCapabilities { allow_privilege: true, ..Default::default() }, ..grant_at(1024) };
        let effective = narrow_effective(&request, &grant, &request_at(1024));
        assert!(!effective.privileged, "grant must never widen the request");
    }
```

2. Confirm FAIL → `cannot find function narrow_effective`.

3. Implement. `narrow_effective` takes the minimum for `memory_limit_bytes`, `pids_limit`, and `cpu_weight`; `privileged` is the logical AND of all three; `network_mode` takes the most restrictive of the three; mounts are intersected so a mount survives only if `read_only` in every input; `image_ref` and `image_manifest_digest` come from the request, and mismatch against the grant is the caller's rejection, not a silent substitution. Document each rule on the function.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox-domain): narrow governed requests to the signed ceiling"`

### Task 8: Define the trust store schema and snapshot

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/trust_store.rs`, `crates/minibox-domain/src/lib.rs`
**Run**: `cargo nextest run -p minibox-domain trust_store`

1. Write the failing test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, key_byte: u8) -> TrustStoreEntry {
        TrustStoreEntry {
            id: id.to_string(),
            public_key: VerifyingKey([key_byte; 32]),
            scope: AuthorityScope::default(),
            policy_digest: [0u8; 32],
            not_before: SystemTime::UNIX_EPOCH,
            not_after: SystemTime::UNIX_EPOCH + Duration::from_secs(86_400),
            revoked: false,
        }
    }

    #[test]
    fn duplicate_ids_are_rejected() {
        let entries = vec![entry("a", 1), entry("a", 2)];
        assert_eq!(
            TrustStoreSnapshot::build(entries),
            Err(TrustStoreError::DuplicateId)
        );
    }

    #[test]
    fn aliased_keys_under_different_ids_are_rejected() {
        let entries = vec![entry("a", 1), entry("b", 1)];
        assert_eq!(
            TrustStoreSnapshot::build(entries),
            Err(TrustStoreError::AliasedKey)
        );
    }

    #[test]
    fn lookup_finds_a_loaded_key() {
        let snap = TrustStoreSnapshot::build(vec![entry("a", 1)]).expect("build");
        assert!(snap.lookup("a").is_some());
        assert!(snap.lookup("missing").is_none());
    }
}
```

2. Confirm FAIL → `cannot find type TrustStoreEntry`.

3. Implement `TrustStoreEntry`, `TrustStoreError` (variants `NotRegularFile`, `SymlinkRejected`, `WrongOwner`, `GroupOrWorldWritable`, `WritableAncestor`, `IdMismatch`, `DuplicateId`, `AliasedKey`, `MalformedEntry`), and `TrustStoreSnapshot` with `build` and `lookup`. `build` checks duplicate ids first, then duplicate public keys. Register the module and re-export alphabetically in `lib.rs`, between `pub mod state;` and `pub mod workflow;`.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox-domain): add governed trust store schema"`

### Task 9: Implement the filesystem trust store loader

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/governed/mod.rs`, `crates/minibox/src/daemon/governed/trust_store.rs`, `crates/minibox/tests/trust_store_load_tests.rs`
**Run**: `cargo nextest run -p minibox trust_store_load`

1. Write the failing tests. Cover, each in its own `#[test]`: a plain entry loads; a symlinked entry returns `SymlinkRejected`; a group-writable file returns `GroupOrWorldWritable`; a directory entry returns `NotRegularFile`; a file whose `id` differs from its filename stem returns `IdMismatch`; two files claiming one id return `DuplicateId`.

2. Confirm FAIL → `cannot find function load_trust_store`.

3. Implement `load_trust_store(dir: &std::path::Path) -> Result<TrustStoreSnapshot, TrustStoreError>`. Open each candidate with `std::fs::OpenOptions` after `symlink_metadata` confirms it is not a symlink; read the file; deserialize `TrustStoreEntry`; assert `entry.id == filename stem`; collect and hand to `TrustStoreSnapshot::build`. Check the directory itself is not group- or world-writable before iterating.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox): load and validate the governed trust store from disk"`

### Task 10: Define ledger record types and phases

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/governed_ledger.rs`, `crates/minibox-domain/src/lib.rs`
**Run**: `cargo nextest run -p minibox-domain governed_ledger`

1. Write the failing test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phases_advance_only_forward() {
        assert!(ExecutionPhase::Reserved.next() == Some(ExecutionPhase::Preparing));
        assert!(ExecutionPhase::Running.next() == Some(ExecutionPhase::Finalizing));
        assert!(ExecutionPhase::Finalized.next().is_none());
        assert!(ExecutionPhase::CleanedUp.next().is_none());
    }

    #[test]
    fn cleanup_is_reachable_from_every_non_terminal_phase() {
        for p in ExecutionPhase::non_terminal() {
            assert!(p.next().is_some(), "{p:?} must be able to advance");
        }
    }
}
```

2. Confirm FAIL → `cannot find type ExecutionPhase`.

3. Implement `ExecutionPhase` with `next()` and `non_terminal()`, plus `ResourceKind`, `ResourceLocator`, `OwnershipSelector`, `ReservationClaim`, `LedgerRecord`, `HeadDigest`, `LedgerError`. Register in `lib.rs` after `pub mod governed_envelope;`. Re-export `ExecutionPhase`, `ExecutionPhaseIter`.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox-domain): add governed ledger record and phase types"`

### Task 11: Implement nonce and execution-id reservation

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/governed/store.rs`, `crates/minibox/tests/governed_ledger_tests.rs`
**Run**: `cargo nextest run -p minibox governed_ledger`

1. Write the failing tests: reserving a fresh nonce succeeds; reserving the same nonce twice returns `LedgerError::DuplicateNonce`; reserving the same `execution_id` twice returns `LedgerError::DuplicateExecution`; two concurrent reservations with one nonce yield exactly one success.

2. Confirm FAIL → `cannot find type FileGovernedLedger`.

3. Implement `FileGovernedLedger` holding the ledger directory and an in-process `tokio::sync::Mutex` guard. `reserve` reads the current records under the lock, scans for a matching nonce or execution id, returns the corresponding error if found, otherwise writes a `Reserved` record using temp-write, `sync_all`, `rename`, and directory `sync_all`.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox): reserve governed nonces atomically"`

### Task 12: Implement hash-chained CAS append

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/governed/store.rs`
**Run**: `cargo nextest run -p minibox governed_ledger`

1. Write the failing tests: appending with the correct head succeeds and advances the head; appending with a stale head returns `LedgerError::CasMismatch`; two writers racing on one head produce exactly one success and one `CasMismatch`; the head equals the SHA-256 of the previous head concatenated with the serialized record.

2. Confirm FAIL → the `append` method is missing.

3. Implement `append`. Serialize the record, compute `HeadDigest` as `sha256(prev_head_bytes || record_bytes)` using the existing `sha2` dependency, compare against `expected_head` under the in-process lock, and write with the same durability sequence as `reserve`.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox): append governed ledger records under a hash-chain CAS"`

### Task 13: Implement interrupted-record recovery

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/governed/store.rs`
**Run**: `cargo nextest run -p minibox governed_ledger`

1. Write the failing tests: `list_interrupted` returns only non-terminal records; running recovery twice is idempotent; a record whose observed locators disagree with its selector is reported as unowned rather than cleaned.

2. Confirm FAIL → `list_interrupted` missing.

3. Implement `list_interrupted` filtering on `ExecutionPhase::is_terminal()`, and `recover` walking each interrupted record, resolving ownership, cleaning, then appending a terminal record under CAS. Make every step idempotent so a second call is a no-op.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox): recover interrupted governed ledger records idempotently"`

### Task 14: Define the `AuthorizationVerifier` port

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/governed_authorization.rs`, `crates/minibox-domain/src/lib.rs`
**Run**: `cargo nextest run -p minibox-domain governed_authorization`

1. Write the failing test:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    struct Stub(bool);

    impl AuthorizationVerifier for Stub {
        fn verify(&self, _e: &GovernedEnvelope, _now: SystemTime) -> Result<VerifiedGrant, AuthorizationError> {
            if self.0 { Ok(VerifiedGrant { key_id: "k".into(), subject: "s".into(), scope: AuthorityScope::default(), effective: unreachable_request() }) } else { Err(AuthorizationError::UnknownKey) }
        }

        fn identifier(&self) -> &'static str { "stub" }
    }

    #[test]
    fn the_port_is_usable_through_a_trait_object() {
        let v: &dyn AuthorizationVerifier = &Stub(true);
        assert_eq!(v.identifier(), "stub");
    }
}
```

2. Confirm FAIL → `cannot find trait AuthorizationVerifier`.

3. Implement `AuthorizationVerifier`, `VerifiedGrant`, and `AuthorizationError` (variants `UnknownKey`, `RevokedKey`, `OutsideValidityWindow`, `SignatureInvalid`, `AudienceMismatch`, `ScopeInsufficient`, `PolicyDigestMismatch`, `ClockSkewExceeded`). Register in `lib.rs`.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox-domain): add the governed authorization verifier port"`

### Task 15: Implement `Ed25519AuthorizationVerifier`

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/governed/authorization.rs`
**Run**: `cargo nextest run -p minibox governed_authorization`

1. Write the failing tests, one per rejection: tampered payload yields `SignatureInvalid`; unknown `key_id` yields `UnknownKey`; a revoked key yields `RevokedKey`; `now` past `not_after` yields `OutsideValidityWindow`; a mismatched `audience` yields `AudienceMismatch`. Add a tampered-but-well-formed test asserting the error is `SignatureInvalid` rather than `UnknownKey`, proving no envelope field is trusted before verification.

2. Confirm FAIL → cannot find `Ed25519AuthorizationVerifier`.

3. Implement against a `TrustStoreSnapshot`. Order the checks so `canonical_signing_bytes` then `verify_strict` run **before** any trust-store lookup, and document that ordering on the function.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox): verify governed envelopes against a trust store snapshot"`

### Task 16: Add the `GovernedRun` protocol variant

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/protocol.rs`
**Run**: `cargo nextest run -p minibox-core protocol`

1. Write the failing test in the existing protocol test module:

```rust
#[test]
fn governed_run_roundtrips_and_ordinary_run_is_unchanged() {
    let req = DaemonRequest::GovernedRun {
        envelope: serde_json::json!({"schema_version": 1, "key_id": "k"}).to_string(),
    };
    let s = serde_json::to_string(&req).expect("serialise");
    let back: DaemonRequest = serde_json::from_str(&s).expect("deserialise");
    assert_eq!(req, back);

    let ordinary = DaemonRequest::Run { /* existing fixture fields */ };
    let s2 = serde_json::to_string(&ordinary).expect("serialise");
    assert!(s2.contains("\"Run\""), "ordinary Run shape must not change");
}
```

2. Confirm FAIL → `no variant named GovernedRun`.

3. Add the variant to `DaemonRequest` and the governed terminal and output variants to `DaemonResponse`. **Audit every terminal-response decision in the dispatch path** — `DaemonResponse::ContainerOutput` is non-terminal while most other variants end streaming (`CLAUDE.md:98`), so the governed output variant must be classified as non-terminal explicitly or governed requests will hang or truncate.

4. Confirm GREEN — including every pre-existing protocol fixture.

5. Commit: `git commit -m "feat(minibox-core): add the governed run protocol variant"`

### Task 17: Reject duplicate JSON keys and enforce the frame limit

**Crate**: `minibox-core`
**File(s)**: `crates/minibox-core/src/protocol.rs`
**Run**: `cargo nextest run -p minibox-core frame`

1. Write the failing tests: `{"a":1,"a":2}` is rejected; a duplicate nested three levels deep is rejected; no duplicate is accepted; a frame over 1 MiB is rejected.

2. Confirm FAIL → duplicate keys are silently accepted today.

3. Implement a recursive scan over `serde_json::Value` — rejecting duplicates must happen **before** typed deserialization, because `serde_json` otherwise keeps the last occurrence and admits a parser-differential attack. Add the 1 MiB check in the client read path.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox-core): reject duplicate frame keys and bound frame size"`

### Task 18: Add defaulted `staged_spawn` and `release` to `ContainerRuntime`

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/runtime.rs`
**Run**: `cargo nextest run -p minibox-domain staged`

1. Write the failing test:

```rust
#[cfg(test)]
mod staged_defaults {
    use super::*;
    use std::sync::Arc;

    struct Bare;

    impl ContainerRuntime for Bare {
        fn capabilities(&self) -> RuntimeCapabilities { RuntimeCapabilities::default() }
        async fn spawn_process(&self, _c: &ContainerSpawnConfig) -> Result<SpawnResult> { unimplemented_marker() }
        async fn wait_for_exit(&self, _r: Option<&str>, _p: u32) -> Result<i32> { unimplemented_marker() }
    }

    #[tokio::test]
    async fn adapters_that_do_not_opt_in_fail_closed() {
        let rt: Arc<dyn ContainerRuntime> = Arc::new(Bare);
        let err = rt
            .staged_spawn(&crate::runtime::ContainerSpawnConfig::default_for_test(), &ResourceIdentity { execution_id: "e".into(), resource_kind: ResourceKind::StagedProcess, tag: "t".into() })
            .await
            .expect_err("must fail closed");
        assert!(matches!(err, StagedError::Unsupported));
    }
}
```

Adapt the fixture construction to whatever constructor `ContainerSpawnConfig` actually exposes — it has no `Default`, so build it with explicit fields or a `#[cfg(test)]` helper.

2. Confirm FAIL → no method `staged_spawn`.

3. Add `staged_spawn` and `release` as `async fn`s with `StagedError::Unsupported` default bodies, plus `ResourceIdentity`, `StagedSpawnResult`, and `StagedError`. All 13 existing implementors across `minibox-core`, `minibox`, and `macbox` must compile unchanged.

4. Confirm GREEN, then `cargo check --workspace` to prove all 13 impls still build.

5. Commit: `git commit -m "feat(minibox-domain): add defaulted staged spawn and release to the runtime port"`

### Task 19: Override staged spawn in the native Linux adapter

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/adapters/runtime.rs`
**Run**: `cargo nextest run -p minibox staged`

1. Write the failing test: a native staged spawn returns a handle whose `released` is `false`; after `release`, `released` is `true`; a second `release` returns `StagedError::SpawnFailed` rather than releasing twice.

2. Confirm FAIL → `LinuxNamespaceRuntime` has no `staged_spawn`.

3. Implement in `LinuxNamespaceRuntime`. Reuse the barrier pipe already created at `crates/minibox/src/container/process.rs:131` — the parent writes one byte to release. Extend `spawn` to take an optional release gate: when `None`, write immediately after `uid_map` installation exactly as today, which keeps ordinary `Run` timing unchanged.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox): stage native spawn behind a release latch"`

### Task 20: Extract the shared `RunEngine`

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handler/run/engine.rs`, `crates/minibox/src/daemon/handler/run.rs`, `crates/minibox/tests/run_characterization_tests.rs`
**Run**: `cargo nextest run -p minibox run_characterization`

1. **Write the characterization tests first.** Cover: argument assembly, env merging, network mode selection, mount ordering, and resource-limit mapping. These must pass against the **unrefactored** handler before any extraction.

2. Confirm they pass. This is the gate that makes the refactor safe — do not proceed until green.

3. Extract normalize, prepare, allocate, staged spawn, release, wait, discover, and cleanup into `run/engine.rs` as `RunEngine`. Re-point the existing `Run` handler at it without changing behavior.

4. Re-run the characterization tests — all must still pass.

5. Commit: `git commit -m "refactor(minibox): extract the shared run engine from the Run handler"`

### Task 21: Wire governed admission into the daemon

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/governed/mod.rs`, `crates/minibox/src/daemon/handler/governed.rs`, `crates/minibox/src/daemon/server.rs`, `crates/minibox/src/daemon/state.rs`
**Run**: `cargo nextest run -p minibox governed_admission`

1. Write the failing tests: a valid envelope reaches `RunEngine`; an expired envelope never allocates; a replayed nonce is rejected at `reserve`; the effective request is narrower than the grant; an unknown key never reaches `RunEngine`.

2. Confirm FAIL → no `GovernedRun` handler.

3. Implement the handler. Reserve **before** allocation, drive `RunEngine`, journal PID identity and `Started` durably, then call `release`, wait, discover, finalize by CAS, clean up. Gate on the adapter being `AdapterSuite::Native`; every other adapter must fail closed.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox): admit governed envelopes through the shared run engine"`

### Task 22: Enforce the local policy ceiling as deny-only

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/governed/policy.rs`, `crates/minibox/tests/governed_policy_tests.rs`
**Run**: `cargo nextest run -p minibox governed_policy`

1. Write the failing tests, plus a proptest: for any request, grant, and policy, the effective limits are never greater than the request's; a policy that denies yields no effective request; a policy can never widen a grant.

2. Confirm FAIL → no `narrow_effective` integration.

3. Implement by building an `ExecutionManifest` from the effective request and calling the existing `PolicyEvaluator::evaluate`. A non-`Allow` decision rejects. **Never widen** — only narrow.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "feat(minibox): apply the local policy as a deny-only governed ceiling"`

### Task 23: Prove secrets never reach envelopes, ledger, errors, or logs

**Crate**: `minibox`
**File(s)**: `crates/minibox/tests/governed_secret_leak_tests.rs`
**Run**: `cargo nextest run -p minibox governed_secret`

1. Write the failing test: submit an envelope carrying a marker value in custom env; assert the marker appears in none of the serialized ledger record, the error string, the protocol response, or captured tracing output.

2. Confirm FAIL → env values are currently recorded verbatim.

3. Make output evidence digest-only. Store a SHA-256 digest of output rather than output itself, and redact configured secret-bearing env keys from every error path.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "fix(minibox): keep governed secrets out of envelopes, ledger, and logs"`

### Task 24: Make skipped privileged suites fail the VM run

**Crate**: `xtask`
**File(s)**: `xtask/src/test_in_vm.rs`, `xtask/src/test_in_vm/tests.rs`
**Run**: `cargo test -p xtask test_in_vm`

Provisioning is **already implemented** — `cargo xtask setup-test-vm` exists (`xtask/src/main.rs:58,125,127`, `xtask/src/setup_test_vm.rs:1`) and the persistent path already fails fast when the machine will not start (`xtask/src/test_in_vm.rs:119`). Do not reimplement it. This task closes the remaining silent-skip defect instead.

1. Write the failing test: when the selected backend is unprivileged, a run that was asked for a privileged suite must exit non-zero. Today `xtask/src/test_in_vm.rs:554-556` prints `(skipping cgroup_tests/system_tests/sandbox_tests — unprivileged backend)` and then reports success — the three privileged suites never run and nothing reflects that in the exit code.

```rust
#[test]
fn unprivileged_backend_fails_when_privileged_suites_are_requested() {
    let outcome = run_script_with_backend(
        Backend::Smolvm,
        &["--exact", "test_e2e_dind_pull_and_run"],
    );
    assert!(!outcome.success, "silently skipping must not report success");
    assert!(outcome.stderr.contains("privileged"), "must name the reason");
}
```

2. Confirm FAIL — the run currently exits 0 after the skip message.

3. Implement. Replace the `println!` at `xtask/src/test_in_vm.rs:554-556` with a hard error naming the skipped suites, the detected backend, and the `cargo xtask setup-test-vm` remedy. Add a `--allow-unprivileged-skip` escape hatch for local convenience runs that records the omission in the summary rather than hiding it.

4. Confirm GREEN, clippy zero warnings.

5. Commit: `git commit -m "fix(xtask): fail the vm test run instead of silently skipping privileged suites"`

### Task 25: Add crash-injection fault points

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/governed/fault.rs`, `crates/minibox/tests/governed_crash_tests.rs`
**Run**: `cargo xtask test-in-vm -- --exact governed_crash`

1. Write the failing tests, one per injection point: after reservation; before and after each allocation; after locator journaling; after staged spawn; after process-identity sync; after `Started` sync; before and after latch release; during output digest finalization; before and after cleanup; before and after terminal CAS. Each asserts recovery converges to a clean terminal state.

2. Confirm FAIL → no fault points exist.

3. Implement `FaultPoint` as a `#[cfg(test)]`-gated hook invoked at each boundary. Make the release latch assert that both identity and `Started` are durable before it fires.

4. These tests run inside the machine provisioned by Task 24 and need the harness fix from Task 26 to filter correctly. Both must be green before this suite is trusted; do not mark this task done on a skipped suite.

5. Commit: `git commit -m "test(minibox): add governed crash-injection fault points"`

### Task 26: Fix the `--exact` filter and stale-binary defects in the VM harness

**Crate**: `xtask`
**File(s)**: `xtask/src/test_in_vm.rs`
**Run**: `cargo test -p xtask test_in_vm`

1. Write the failing tests: passing `--exact some_test` must restrict invocation to the suite that actually contains that test rather than appending the filter to all five; a stale second `sandbox_tests-<hash>` binary must not be executed.

2. Confirm FAIL — current code at `xtask/src/test_in_vm.rs:637-640` appends the same args to every suite, and `:566-585` accepts every deps-dir entry matching a suite prefix.

3. Implement. Restrict the filter to the suite whose binary contains the test name, and select only the newest binary per suite prefix by modification time.

4. Confirm GREEN.

5. Commit: `git commit -m "fix(xtask): scope --exact filters and stop running stale test binaries"`

### Task 27: Add Kani proofs for the ceiling properties

**Crate**: `minibox-domain`
**File(s)**: `crates/minibox-domain/src/governed_envelope.rs`
**Run**: `cargo kani --package minibox-domain`

1. Write the failing proofs: `narrow_effective` never returns a memory limit greater than any input's; `narrow_effective` never sets `privileged` when the request did not; `check_bounds` rejects every value above `MAX_EXACT_JSON_INTEGER`.

2. Confirm FAIL → no proofs declared.

3. Implement the `#[cfg(kani)] mod kani_proofs` block, matching the existing style in `crates/minibox-domain/src/execution_policy.rs:654-738`. `cargo kani` is gated by `check-cfg` in `Cargo.toml:114`.

4. Confirm GREEN.

5. Commit: `git commit -m "test(minibox-domain): prove governed ceiling monotonicity with Kani"`

### Task 28: Full gate

**Crate**: `minibox`
**File(s)**: none
**Run**: `cargo xtask verify`

1. `cargo fmt --all`
2. `cargo clippy --workspace -- -D warnings` → zero warnings
3. `cargo xtask verify` → fmt check, workspace check, clippy, borrow fixtures, docs audit
4. `cargo xtask test unit`
5. Confirm every existing protocol fixture still passes — ordinary `Run` must be byte-identical.

6. Commit: `git commit -m "chore(minibox): governed execution full gate"`

---

## Plan Self-Check

- [x] **Every requirement maps to at least one task.** Design sections and their tasks: envelope types → 1–2 · canonicalization → 3, 4, 6 · signing → 5 · narrowing → 7 · trust store → 8–9 · ledger → 10–13 · verifier port and impl → 14–15 · protocol → 16–17 · staged release → 18–19 · engine extraction → 20 · admission → 21 · policy ceiling → 22 · secret hygiene → 23 · crash campaigns → 24 · smolvm provisioning → 25 · harness defects → 26 · formal verification → 27 · gate → 28.
- [x] **No placeholders.** Every task names exact files, exact symbols, and exact expected failures.
- [x] **Names consistent across tasks.** `GovernedEnvelope`, `Grant`, `AuthorityScope`, `GovernedRequest`, `ResourceLimits`, `GrantedCapabilities`, `Signature`, `VerifyingKey`, `TrustStoreEntry`, `TrustStoreSnapshot`, `TrustStoreError`, `ExecutionPhase`, `LedgerRecord`, `ReservationClaim`, `HeadDigest`, `LedgerError`, `AuthorizationVerifier`, `VerifiedGrant`, `AuthorizationError`, `ResourceIdentity`, `StagedSpawnResult`, `StagedError`, `FileGovernedLedger`, `Ed25519AuthorizationVerifier`, `load_trust_store`, `canonical_signing_bytes`, `canonicalize_json`, `narrow_effective`, `validate_structure`.
- [x] **Each task is 2–5 minutes of focused work.** Tasks 19, 20, 21, and 24 are the largest and may exceed that; they are the natural split points if they run long.
- [x] **Every task ends with a commit.**
- [x] **Dependency order respected.** Domain types precede implementations; `RunEngine` extraction (20) precedes governed admission (21); smolvm provisioning (25) precedes the crash campaigns it gates (24 is written before it runs).

## Known Gaps Carried Into Implementation

1. **Task 24 cannot pass until Task 25 lands.** The crash-injection campaigns need the provisioned smolvm machine. Written before it runs so the fault points are designed, but it must not be marked done on a skipped suite.
2. **The source plan's preconditions are stale.** `.ctx/godmode/plans/2026-09-22-signed-governed-execution-daemon.md` points at `docs/designs/2026-09-22-signed-governed-execution-daemon-design.md`, which does not exist, and precondition 2 ("pin an immutable Crux release") is superseded — this design owns the schema.
3. **Task `…-t1` blocker text is stale** and should be reworded to "define the envelope contract and generate golden vectors" before unblocking.
4. **`govexec-1` duplicates `…-t1`** in `minibox-core`; both remain pending.
5. **`docs_lint.rs:21-22` does not cover `docs/designs/`**, so design-doc frontmatter is currently ungated. The approved doc uses `status: approved`, which is in `ALLOWED_STATUSES`, so it would pass if the scope widens.
