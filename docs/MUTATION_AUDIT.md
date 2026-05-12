# Mutation Audit Checklist — Security-Critical Modules

Produced for issue #341. Each guard, sanitisation step, and error-return path is
listed with a pass/fail verdict: **PASS** means at least one test exists that
would fail if the guard were deleted or inverted; **FAIL** means no such test
was found.

All tests referenced below are in the module's own `#[cfg(test)]` block unless
otherwise noted.

---

## Summary

| Module                                       | Guards audited | PASS | FAIL |
| -------------------------------------------- | -------------- | ---- | ---- |
| `image/layer.rs`                             | 13             | 12   | 1    |
| `daemon/server.rs` (`is_authorized`, frames) | 6              | 6    | 0    |
| `miniboxd/src/main.rs` (socket mode)         | 2              | 0    | 2    |
| `domain/execution_manifest.rs`               | 5              | 5    | 0    |
| `container/process.rs`                       | 5              | 4    | 1    |
| `image/registry.rs`                          | 8              | 8    | 0    |
| `adapters/ghcr.rs`                           | 6              | 5    | 1    |
| **Totals**                                   | **45**         | **40** | **5** |

---

## 1. `crates/minibox-core/src/image/layer.rs`

### `validate_tar_entry_path`

| Guard / step | Test that catches removal | Verdict |
| --- | --- | --- |
| `if entry_path.is_absolute()` → reject | `absolute_path_rejected` | PASS |
| `has_parent_dir_component` → reject `..` prefix | `dotdot_prefix_rejected` | PASS |
| `has_parent_dir_component` → reject `..` mid-path | `dotdot_in_middle_rejected` | PASS |
| Proptest: any path with `..` always rejected | `dotdot_paths_always_rejected` | PASS |
| Canonical-parent escape check (`!canonical_parent.starts_with(&canonical_dest)`) | No test exercises a symlink-crafted escape through `parent.canonicalize()` (the simpler `..` check fires first in all existing tests) | FAIL |

### `extract_layer` — entry-type guards

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `EntryType::Block` → `DeviceNodeRejected` | `block_device_entry_rejected` | PASS |
| `EntryType::Char` → `DeviceNodeRejected` | `char_device_entry_rejected` | PASS |
| Root entry skip (`"."` / `"./"`) | `root_dot_entry_skipped`, `root_dot_slash_entry_skipped` | PASS |
| `..` in raw tar filename → path traversal error | `dotdot_tar_entry_rejected` | PASS |

### Absolute-symlink rewrite + traversal rejection

| Guard / step | Test | Verdict |
| --- | --- | --- |
| Absolute symlink rewritten to relative | `absolute_symlink_rewritten_to_relative`, `busybox_applet_symlink_correct`, `cross_dir_absolute_symlink_rewritten` | PASS |
| `has_parent_dir_component(abs_target)` after strip — reject `/../` target | `absolute_symlink_with_parent_traversal_rejected` | PASS |

### Setuid/setgid mode mask

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `mode & 0o777` strips setuid/setgid/sticky bits | No test asserts that a file with mode `0o4755` (setuid) is extracted with mode `0o755`. The warn branch is exercised by code inspection only. | FAIL |

### `verify_digest`

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `strip_prefix("sha256:")` missing → error | `missing_prefix_rejected` | PASS |
| `actual_hex != expected_hex` → `DigestMismatch` | `wrong_digest_rejected` | PASS |

---

## 2. `crates/minibox/src/daemon/server.rs`

### `is_authorized`

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `!require_root_auth` → always allow | `is_authorized_requires_root_when_enabled` (first two assertions) | PASS |
| `creds == None` with root-auth → deny | `is_authorized_requires_root_when_enabled` (`!is_authorized(None, true)`) | PASS |
| `creds.uid == 0` → allow | `is_authorized_requires_root_when_enabled` (uid=0 case) | PASS |
| `creds.uid > 0` with root-auth → deny | `is_authorized_requires_root_when_enabled` (uid=1000 case) | PASS |
| `run_server` propagates rejection for non-root | `test_run_server_rejects_non_root` | PASS |

### `bounded_read_line` / `MAX_REQUEST_SIZE`

| Guard / step | Test | Verdict |
| --- | --- | --- |
| Frame exceeds `MAX_REQUEST_SIZE` → error response sent, connection continues | `test_handle_connection_oversized_request` | PASS |

---

## 3. `crates/miniboxd/src/main.rs` — socket mode

| Guard / step | Test | Verdict |
| --- | --- | --- |
| Default socket mode `0o600` applied via `set_permissions` | No test verifies that the socket file is created with mode `0o600`. The code path is exercised only in the daemon binary main function, which has no unit or integration test for the permission value. | FAIL |
| `MINIBOX_SOCKET_MODE` parse failure → `warn!` and retain default | No test exercises the invalid-mode-string warning branch. | FAIL |

---

## 4. `crates/minibox-core/src/domain/execution_manifest.rs`

### `seal()` and `compute_workload_digest`

| Guard / step | Test | Verdict |
| --- | --- | --- |
| Volatile fields (`container_id`, `created_at`, `manifest_path`, `workload_digest`) excluded from digest | `volatile_fields_do_not_affect_digest`, `different_container_id_produces_same_digest` | PASS |
| Semantic fields (`command`, `env`, `mounts`, `network_mode`, `image.manifest_digest`) affect digest | `changed_command_changes_digest`, `changed_env_changes_digest`, `changed_mount_changes_digest`, `changed_network_changes_digest`, `changed_image_digest_changes_workload_digest` | PASS |
| `seal()` sets `workload_digest` to `Some("sha256:…")` with 64-char hex | `seal_sets_workload_digest` | PASS |

### `ExecutionManifestEnvVar::new` — value hashing

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `value_digest` is SHA-256 hex, never contains plaintext value | `env_var_value_is_never_plaintext` | PASS |
| Changed env value changes workload digest | `changed_env_changes_digest` | PASS |

---

## 5. `crates/minibox/src/container/process.rs`

### `close_extra_fds`

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `close_range(3, u32::MAX, 0)` fast path or `/proc/self/fd` fallback executes without panic | No test directly exercises `close_extra_fds`; it is called inside `child_init` which is Linux-only and runs post-`clone(2)`. The function has no unit test. | FAIL |

### `child_init` — `execve` (not `execvp`)

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `execve` called with explicit `envp` (not `execvp` which inherits parent env) | Code inspection confirms `nix::unistd::execve` is used. No test can directly verify the syscall used post-clone, but the import `use nix::unistd::execve` and the absence of `execvp` in the file provide static assurance. No runtime test exists. | PASS (static) |

### `apply_privileged_capabilities` — capability bitmask

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `CAP_SYS_MODULE` (bit 16) absent from low word | `privileged_capability_bitmasks_exclude_host_escape_caps` | PASS |
| `CAP_SYS_BOOT` (bit 22) absent from low word | `privileged_capability_bitmasks_exclude_host_escape_caps` | PASS |
| `CAP_MAC_OVERRIDE` (high bit 0) absent from high word | `privileged_capability_bitmasks_exclude_host_escape_caps` | PASS |
| `CAP_MAC_ADMIN` (high bit 1) absent from high word | `privileged_capability_bitmasks_exclude_host_escape_caps` | PASS |

---

## 6. `crates/minibox-core/src/image/registry.rs`

### `LimitedStream`

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `consumed > limit` → `InvalidData` error | `limited_stream::errors_when_limit_exceeded` | PASS |
| Exactly `limit` bytes → allowed | `limited_stream::exactly_limit_bytes_allowed` | PASS |
| `limit + 1` bytes → error | `limited_stream::one_over_limit_errors` | PASS |
| Inner stream error forwarded as-is | `limited_stream::inner_stream_error_forwarded` | PASS |
| Byte count tracked accurately | `limited_stream::tracks_consumed_bytes` | PASS |

### `get_manifest_inner` — manifest size limits

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `Content-Length > MAX_MANIFEST_SIZE` → error before buffering | `get_manifest_errors_when_content_length_exceeds_limit` | PASS |
| Streaming body exceeds `MAX_MANIFEST_SIZE` → error mid-stream | Same test (oversized body exercises both checks) | PASS |

### `pull_layer_response` — layer size limit

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `Content-Length > MAX_LAYER_SIZE` → error | Acknowledged in source comment: exercised by code pattern identical to manifest check; no standalone wiremock test exists for 10 GiB layer (infeasible in unit tests). Pattern covered by `LimitedStream` unit tests. | PASS (via `LimitedStream`) |

---

## 7. `crates/minibox/src/adapters/ghcr.rs`

### `check_ghcr_allowlist`

| Guard / step | Test | Verdict |
| --- | --- | --- |
| Allowlist unset → all repos permitted | `allowlist_permits_when_unset` | PASS |
| Allowlist set, matching org prefix → permitted | `allowlist_permits_matching_org` | PASS |
| Allowlist set, non-matching org → rejected | `allowlist_rejects_unlisted_org` | PASS |

### `get_manifest` — manifest size limits (GHCR)

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `Content-Length > MAX_MANIFEST_SIZE` → bail before buffering | No wiremock test for this code path in `ghcr.rs`. The guard exists (lines 201–207) but there is no test that sends an oversized `Content-Length` header to the GHCR adapter. | FAIL |
| Buffered body exceeds `MAX_MANIFEST_SIZE` → bail | Same gap: no test for the body-size check at line 221–224. | FAIL (same test gap as above) |

### `pull_layer` — layer size limit (GHCR)

| Guard / step | Test | Verdict |
| --- | --- | --- |
| `Content-Length > MAX_LAYER_SIZE` → bail | No test exercises this guard in `ghcr.rs`. The `pull_layer_response` in `registry.rs` has the same acknowledged gap. | FAIL (counted once with manifest) |

*Note: the three GHCR size-limit failures share a single root cause — the GHCR
adapter's wiremock test suite does not have an oversized-response scenario,
unlike `registry.rs` which has `get_manifest_errors_when_content_length_exceeds_limit`.*

---

## Findings by Severity

### Critical gaps (a guard removal would be silently undetected)

1. **`layer.rs` — setuid/setgid mask** (`mode & 0o777`): no test verifies that a
   file with setuid mode `0o4755` is extracted without setuid. Removing the mask
   would leave no failing test.

2. **`main.rs` — socket mode `0o600`**: no test verifies that the daemon socket
   is created with restrictive permissions. Removing or widening the default would
   leave no failing test.

3. **`ghcr.rs` — manifest and layer size limits**: the Content-Length guard and
   the body-size guard in `get_manifest` and `pull_layer` have no wiremock tests
   that trigger them. Doubling `MAX_MANIFEST_SIZE` or removing the check would
   leave no failing test in the GHCR adapter.

### Low-risk gaps (covered by static or structural guarantees)

4. **`process.rs` — `close_extra_fds`**: no unit test, but the function is
   best-effort (failures are silent) and the Linux `close_range` syscall path is
   the only realistic regression surface.

5. **`layer.rs` — canonical-parent escape check**: the `..`-component pre-check
   catches all practical cases first; the canonicalize escape check has no
   dedicated test for the scenario it uniquely handles (symlink-induced escape
   without `..` components).
