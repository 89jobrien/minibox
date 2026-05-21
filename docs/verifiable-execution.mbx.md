# Verifiable Execution Manifests

This document describes the execution manifest format, how manifests are
produced and stored, the planned attestation path, and how to inspect
manifests for completed container runs.

## 1. ExecutionManifest Structure

Every container run produces an `ExecutionManifest` — a JSON document
that captures every measured input to the run. The canonical Rust type
lives in `minibox-core/src/domain/execution_manifest.rs`.

### Top-Level Fields

| Field              | Type              | In digest? | Description                              |
| ------------------ | ----------------- | :--------: | ---------------------------------------- |
| `schema_version`   | `u32`             |    yes     | Schema version for forward compat (currently `1`). |
| `container_id`     | `string`          |     no     | Unique container identifier.             |
| `created_at`       | `string` (ISO 8601) |   no     | Timestamp of manifest creation.          |
| `manifest_path`    | `string?`         |     no     | Filesystem path where the manifest is persisted. |
| `workload_digest`  | `string?`         |     no     | The sealed workload digest (see below).  |
| `subject`          | `object`          |    yes     | Image identity (see Subject).            |
| `runtime`          | `object`          |    yes     | Runtime configuration (see Runtime).     |
| `request`          | `object`          |    yes     | Original run request parameters.         |

### Subject

| Field                     | Type       | Description                              |
| ------------------------- | ---------- | ---------------------------------------- |
| `image_ref`               | `string`   | Image reference as provided (e.g. `alpine:3.18`). |
| `image.manifest_digest`   | `string?`  | OCI image manifest SHA-256 digest.       |
| `image.config_digest`     | `string?`  | OCI image config SHA-256 digest.         |
| `image.layer_digests`     | `string[]` | Ordered layer content digests.           |

### Runtime

| Field                                | Type       | Description                              |
| ------------------------------------ | ---------- | ---------------------------------------- |
| `command`                            | `string[]` | Command and arguments.                   |
| `env[].name`                         | `string`   | Environment variable name.               |
| `env[].value_digest`                 | `string`   | SHA-256 hex of the variable value (never plaintext). |
| `mounts[].host_path`                 | `string`   | Host-side bind mount path.               |
| `mounts[].container_path`            | `string`   | Container-side mount path.               |
| `mounts[].read_only`                 | `bool`     | Whether the mount is read-only.          |
| `resource_limits.memory_limit_bytes` | `u64?`     | Memory limit in bytes.                   |
| `resource_limits.cpu_weight`         | `u64?`     | CPU weight.                              |
| `network_mode`                       | `string`   | Network isolation mode (e.g. `none`, `host`, `bridge`). |
| `privileged`                         | `bool`     | Whether privileged mode is enabled.      |
| `platform`                           | `string?`  | Requested platform override, if any.     |

### Request

| Field       | Type      | Description                              |
| ----------- | --------- | ---------------------------------------- |
| `name`      | `string?` | Container name if provided by the user.  |
| `ephemeral` | `bool`    | Whether the run was ephemeral (streaming). |

### Serialization Format

Manifests are serialized as pretty-printed JSON. All fields use
`#[serde(default)]` and `#[serde(skip_serializing_if)]` where
appropriate to maintain forward and backward wire compatibility.

## 2. Content-Addressing: The Workload Digest

The workload digest is a deterministic SHA-256 hash computed over a
stable JSON projection of the manifest. The projection includes:

- `schema_version`
- `subject` (full)
- `runtime` (full)
- `request` (full)

The projection deliberately **excludes** volatile/instance-specific
fields: `container_id`, `created_at`, `manifest_path`, and
`workload_digest` itself.

Because serde serializes Rust struct fields in declaration order, the
JSON byte representation is deterministic. Equal semantic inputs always
produce equal digests.

The digest format is `sha256:<64 hex chars>`.

### Sealing

Calling `ExecutionManifest::seal()` computes the digest and writes it
into the `workload_digest` field. This happens before the manifest is
persisted to disk.

## 3. Manifest Production and Storage

The daemon produces the execution manifest **before** the container
process is spawned. The sequence is:

1. The daemon receives a `Run` request.
2. It resolves the image, layers, and runtime configuration.
3. It constructs an `ExecutionManifest` from the resolved inputs.
4. It calls `seal()` to compute and set the workload digest.
5. It writes the manifest to:
   ```
   {containers_base}/{container_id}/execution-manifest.json
   ```
6. Only after the manifest is persisted does the daemon spawn the
   container process.

This ordering guarantees the manifest exists for any container that
has ever started, regardless of whether the process completed
successfully.

### Retrieval via Protocol

Two daemon protocol requests operate on manifests:

- `GetManifest { id }` -- returns the manifest JSON as a
  `DaemonResponse::Manifest`.
- `VerifyManifest { id, policy_path }` -- evaluates the manifest
  against an `ExecutionPolicy` loaded from `policy_path` and returns
  a `DaemonResponse::VerifyResult`.

## 4. Execution Policy

`ExecutionPolicy` (defined in
`minibox-core/src/domain/execution_policy.rs`) evaluates a manifest
against a rule set. Policy rules can constrain:

- Allowed/denied image name patterns
- Network mode restrictions
- Privileged mode gate
- Memory limit cap
- Mount path prefix allowlist

Policies are loaded from JSON files. The `evaluate()` method returns
a `PolicyDecision` (allow or deny with reasons).

## 5. Planned Attestation Path

The manifest format is designed for future integration with
cryptographic attestation. The intended design:

### Signing

1. After `seal()`, the daemon (or an external signer) signs the
   workload digest using a private key.
2. The signature is stored alongside the manifest as
   `execution-manifest.sig` or embedded in an attestation envelope.
3. Supported formats (planned): Sigstore cosign and in-toto
   attestation envelopes.

### Verification Workflow

1. A verifier retrieves the manifest (via `GetManifest` or by
   reading the JSON file directly).
2. The verifier recomputes the workload digest from the manifest
   fields and confirms it matches `workload_digest`.
3. The verifier checks the signature against a trusted public key
   or certificate chain.
4. Optionally, the verifier evaluates an `ExecutionPolicy` to
   confirm the workload configuration meets organizational
   constraints.

### Trust Model

- The daemon is the manifest producer and initial signer.
- The workload digest is the attestation subject: it uniquely
  identifies the semantic workload configuration without binding
  to a specific container instance or timestamp.
- Rotating signing keys does not invalidate existing manifests;
  verification uses the key that was active at signing time.

### Current Status

Signing and signature verification are **not yet implemented**.
The manifest structure, digest computation, and policy evaluation
are implemented and tested. The `workload_digest` field serves as
the future attestation subject.

## 6. Developer Guide: Inspecting Manifests

### Via CLI

```sh
# Print the manifest for a container
mbx manifest <container-id>

# Verify a container against a policy
mbx verify <container-id> --policy policy.json
# Exit code 0 = allowed, 1 = denied
```

### Via Filesystem

Manifests are plain JSON files at a predictable path:

```sh
cat /var/lib/minibox/containers/<container-id>/execution-manifest.json
```

Use `jq` to extract specific fields:

```sh
# Show the workload digest
jq '.workload_digest' \
  /var/lib/minibox/containers/<id>/execution-manifest.json

# Show the command that was run
jq '.runtime.command' \
  /var/lib/minibox/containers/<id>/execution-manifest.json

# List all environment variable names (values are hashed)
jq '[.runtime.env[].name]' \
  /var/lib/minibox/containers/<id>/execution-manifest.json
```

### Verifying Digest Integrity

To confirm a manifest has not been tampered with:

1. Read the manifest JSON.
2. Extract the `workload_digest` field.
3. Zero out the volatile fields (`container_id`, `created_at`,
   `manifest_path`, `workload_digest`) and recompute the SHA-256
   over the projection JSON.
4. Compare with the stored digest.

The Rust API exposes this via `compute_workload_digest()`:

```rust
let manifest: ExecutionManifest = serde_json::from_str(&json)?;
let computed = manifest.compute_workload_digest()?;
let stored = manifest.workload_digest.as_deref().unwrap_or("");
assert_eq!(computed.to_string(), stored);
```

## References

- Type definitions: `crates/minibox-core/src/domain/execution_manifest.rs`
- Policy engine: `crates/minibox-core/src/domain/execution_policy.rs`
- Protocol requests: `GetManifest`, `VerifyManifest` in
  `crates/minibox-core/src/protocol.rs`
- Architecture overview: [`docs/ARCHITECTURE.mbx.md`](ARCHITECTURE.mbx.md)
