# Plan: Rustqual TQ + Crux-Plugin DRY Sweep

## Goal

Fix ~85 TQ_NO_SUT test naming findings across `minibox-core`, `minibox`,
`miniboxd`, and `mbx`; suppress ERROR_HANDLING in mock/test-helper code
via config; and extract a test macro in `minibox-crux-plugin` to eliminate
14 DUPLICATE findings. This covers categories deferred by the 2026-05-22
bulk sweep plan.

## Architecture

- Crates affected: `minibox-core`, `minibox`, `miniboxd`, `mbx`,
  `minibox-crux-plugin`
- No new traits/types — rename-only for TQ_NO_SUT, config for
  ERROR_HANDLING, macro extraction for DUPLICATE
- No public API changes
- Data flow: unchanged

## Tech Stack

- Rust 2024
- No new dependencies

## Tasks

### Task 1: TQ_NO_SUT renames in minibox-core

**Crate**: `minibox-core`
**File(s)**:
- `crates/minibox-core/src/domain.rs`
- `crates/minibox-core/src/protocol.rs`
- `crates/minibox-core/tests/conformance_error_model.rs`
- `crates/minibox-core/tests/protocol_evolution.rs`
**Run**: `cargo nextest run -p minibox-core`

Rename test functions so the name includes the type or function under
test. rustqual traces direct calls; the SUT must appear in the test
name.

The pattern: if a test exercises `DaemonRequest` deserialization, the
test name must contain `daemon_request`. If it exercises
`DomainError` display, the name must contain `domain_error`.

Renames for `src/domain.rs`:

```
test_domain_error_display_image_not_found
  -> domain_error_display_image_not_found

test_domain_error_display_container_not_found
  -> domain_error_display_container_not_found

test_domain_error_display_resource_limit_exceeded
  -> domain_error_display_resource_limit_exceeded

pty_config_json_missing_fields_use_serde_default
  -> pty_config_deserialize_missing_fields_use_serde_default

workflow_step_defaults_continue_on_error_false
  -> workflow_step_deserialize_defaults_continue_on_error_false
```

Renames for `src/protocol.rs` (representative — apply same pattern to
all TQ_NO_SUT findings in this file):

```
run_request_defaults_ephemeral_false
  -> daemon_request_run_defaults_ephemeral_false

run_request_explicit_ephemeral_true
  -> daemon_request_run_explicit_ephemeral_true

container_stopped_roundtrip
  -> daemon_response_container_stopped_roundtrip

run_request_without_network_defaults_to_none_option
  -> daemon_request_run_without_network_defaults_to_none

container_logs_request_follow_defaults_false
  -> daemon_request_container_logs_follow_defaults_false

run_request_old_json_without_mounts_defaults
  -> daemon_request_run_old_json_without_mounts_defaults

run_request_tty_defaults_false
  -> daemon_request_run_tty_defaults_false

wire_snapshot_run_request
  -> daemon_request_wire_snapshot_run

wire_snapshot_stop_request
  -> daemon_request_wire_snapshot_stop

wire_snapshot_pull_request
  -> daemon_request_wire_snapshot_pull

wire_snapshot_container_created_response
  -> daemon_response_wire_snapshot_container_created

wire_snapshot_success_response
  -> daemon_response_wire_snapshot_success

wire_snapshot_error_response
  -> daemon_response_wire_snapshot_error

wire_snapshot_container_stopped_response
  -> daemon_response_wire_snapshot_container_stopped

wire_snapshot_container_stopped_nonzero_exit
  -> daemon_response_wire_snapshot_container_stopped_nonzero_exit

wire_snapshot_push_credentials_basic
  -> push_credentials_wire_snapshot_basic

push_credentials_debug_redacts_password
  -> push_credentials_debug_redacts_password  (already has SUT — verify)

push_credentials_debug_redacts_token
  -> push_credentials_debug_redacts_token  (already has SUT — verify)

wire_format_run_request_field_names_stable
  -> daemon_request_wire_format_run_field_names_stable

pull_without_platform_deserializes
  -> daemon_request_pull_without_platform_deserializes

pull_with_platform_deserializes
  -> daemon_request_pull_with_platform_deserializes

run_without_platform_deserializes
  -> daemon_request_run_without_platform_deserializes

run_with_platform_deserializes
  -> daemon_request_run_with_platform_deserializes

wire_snapshot_update_request
  -> daemon_request_wire_snapshot_update

update_request_defaults_bools_false
  -> daemon_request_update_defaults_bools_false

update_request_all_true
  -> daemon_request_update_all_true

update_request_containers_and_restart
  -> daemon_request_update_containers_and_restart

wire_snapshot_update_progress_response
  -> daemon_response_wire_snapshot_update_progress
```

Renames for `tests/conformance_error_model.rs`:

```
conformance_image_error_not_found_display
  -> conformance_image_error_not_found_display  (has SUT — verify)

conformance_image_error_digest_mismatch_display
  -> conformance_image_error_digest_mismatch_display  (has SUT — verify)

... (apply same check to all conformance_* tests in this file)
```

Renames for `tests/protocol_evolution.rs`:

```
test_request_run_backward_compat_omits_optional_fields
  -> daemon_request_run_backward_compat_omits_optional_fields

test_request_exec_backward_compat_omits_optional_fields
  -> daemon_request_exec_backward_compat_omits_optional_fields

test_request_prune_backward_compat_omits_dry_run
  -> daemon_request_prune_backward_compat_omits_dry_run

test_request_commit_backward_compat_omits_optional_fields
  -> daemon_request_commit_backward_compat_omits_optional_fields

test_request_build_backward_compat_omits_optional_fields
  -> daemon_request_build_backward_compat_omits_optional_fields

test_request_run_pipeline_backward_compat_omits_optional_fields
  -> daemon_request_run_pipeline_backward_compat_omits_optional_fields

run_pipeline_request_snapshot
  -> daemon_request_run_pipeline_snapshot

run_pipeline_request_minimal_snapshot
  -> daemon_request_run_pipeline_minimal_snapshot

pipeline_complete_response_snapshot
  -> daemon_response_pipeline_complete_snapshot
```

1. Apply all renames in the files listed above.
2. Verify:
   ```
   cargo nextest run -p minibox-core    -> all green
   cargo clippy -p minibox-core -- -D warnings  -> zero warnings
   rustqual crates/minibox-core/ --no-fail 2>&1 | grep TQ_NO_SUT  -> 0 hits
   ```
3. Commit: `refactor(minibox-core): rename tests to include SUT for rustqual TQ_NO_SUT`

### Task 2: TQ_NO_SUT renames in minibox

**Crate**: `minibox`
**File(s)**:
- `crates/minibox/src/adapters/network/bridge.rs`
- `crates/minibox/src/container/process.rs`
- `crates/minibox/src/daemon/state.rs`
- `crates/minibox/src/image/layer.rs`
- `crates/minibox/src/image/registry.rs`
- `crates/minibox/src/domain.rs`
**Run**: `cargo nextest run -p minibox`

Renames:

```
# bridge.rs
dnat_destination_format -> bridge_network_dnat_destination_format
dns_fallback_when_config_has_no_servers -> bridge_network_dns_fallback_when_config_has_no_servers
dns_config_used_verbatim_when_non_empty -> bridge_network_dns_config_used_verbatim_when_non_empty

# process.rs
privileged_capability_bitmasks_exclude_host_escape_caps
  -> apply_privileged_capabilities_bitmasks_exclude_host_escape_caps

# state.rs
container_record_deserializes_without_creation_params
  -> container_record_deserialize_without_creation_params

# layer.rs
exhaustive_setuid_mask_strips_all_special_bits
  -> setuid_mask_strips_all_special_bits_exhaustive

# registry.rs
test_constants_manifest_size -> manifest_size_limit_within_bounds
test_constants_layer_size -> layer_size_limit_within_bounds
test_constants_total_image_size -> total_image_size_limit_within_bounds

# domain.rs
test_domain_error_display_image_not_found -> domain_error_display_image_not_found
test_domain_error_display_container_not_found -> domain_error_display_container_not_found
test_domain_error_display_resource_limit_exceeded -> domain_error_display_resource_limit_exceeded
```

1. Apply all renames.
2. Verify:
   ```
   cargo nextest run -p minibox    -> all green
   cargo clippy -p minibox -- -D warnings  -> zero warnings
   rustqual crates/minibox/ --no-fail 2>&1 | grep TQ_NO_SUT  -> 0 hits
   ```
3. Commit: `refactor(minibox): rename tests to include SUT for rustqual TQ_NO_SUT`

### Task 3: TQ_NO_SUT renames in miniboxd and mbx

**Crate**: `miniboxd`, `mbx`
**File(s)**:
- `crates/miniboxd/src/config.rs`
- `crates/mbx/src/commands/exec.rs`
- `crates/mbx/src/commands/logs.rs`
- `crates/mbx/src/commands/run.rs`
- `crates/mbx/tests/conformance_cli.rs`
**Run**: `cargo nextest run -p miniboxd -p mbx`

Renames for `miniboxd/src/config.rs`:

```
empty_toml_produces_defaults -> daemon_config_empty_toml_produces_defaults
parses_full_config -> daemon_config_parses_full_config
invalid_toml_returns_defaults -> daemon_config_invalid_toml_returns_defaults
```

Renames for `mbx/src/commands/exec.rs`:

```
exec_request_has_type_tag -> daemon_request_exec_has_type_tag
exec_started_response_deserialises -> daemon_response_exec_started_deserialises
exec_output_chunk_decodes -> daemon_response_exec_output_chunk_decodes
```

Renames for `mbx/src/commands/logs.rs`:

```
logs_request_has_type_tag -> daemon_request_logs_has_type_tag
logs_request_follow_field -> daemon_request_logs_follow_field
log_line_response_deserialises -> daemon_response_log_line_deserialises
```

Renames for `mbx/src/commands/run.rs`:

```
decode_output_chunk -> daemon_response_decode_output_chunk
decode_stderr_chunk -> daemon_response_decode_stderr_chunk
```

Renames for `mbx/tests/conformance_cli.rs`:

```
conformance_cli_no_args_shows_help -> conformance_minibox_cli_no_args_shows_help
conformance_cli_help_flag -> conformance_minibox_cli_help_flag
conformance_cli_version_flag -> conformance_minibox_cli_version_flag
conformance_cli_unknown_subcommand_fails -> conformance_minibox_cli_unknown_subcommand_fails
conformance_protocol_request_variants_serialize
  -> conformance_daemon_request_variants_serialize
conformance_protocol_response_variants_serialize
  -> conformance_daemon_response_variants_serialize
```

1. Apply all renames.
2. Verify:
   ```
   cargo nextest run -p miniboxd -p mbx    -> all green
   rustqual crates/miniboxd/ --no-fail 2>&1 | grep TQ_NO_SUT  -> 0 hits
   rustqual crates/mbx/ --no-fail 2>&1 | grep TQ_NO_SUT  -> 0 hits
   ```
3. Commit: `refactor(miniboxd,mbx): rename tests to include SUT for rustqual TQ_NO_SUT`

### Task 4: Suppress ERROR_HANDLING in mock/test-helper files

**Crate**: `minibox-core`, `minibox`, `minibox-testsuite`
**File(s)**:
- `crates/minibox-core/rustqual.toml` (create if absent)
- `crates/minibox/rustqual.toml`
- `crates/minibox-testsuite/rustqual.toml` (create if absent)
**Run**: `rustqual crates/minibox-core/ --no-fail; rustqual crates/minibox/ --no-fail; rustqual crates/minibox-testsuite/ --no-fail`

Mock files use `.unwrap()` by design — test code should `expect()` or
`unwrap()`, not propagate errors. Suppress by excluding mock/test
files from error-handling detection.

For `crates/minibox-core/rustqual.toml` — add if not present:

```toml
[complexity]
detect_error_handling = false
```

This is acceptable because minibox-core's ERROR_HANDLING findings are
all in `src/adapters/mocks.rs` (14 findings). The alternative is
per-function suppression, but mock impls are inherently unwrap-heavy.

For `crates/minibox/rustqual.toml` — append:

```toml
# Mock and test helper files use .unwrap()/.expect() by design.
# ERROR_HANDLING findings in these files are false positives.
[complexity]
detect_error_handling = false
```

Wait — this would suppress real error-handling findings in production
code too. Better approach: exclude the specific files.

For all three crates, add mock/test files to `exclude_files`:

`crates/minibox-core/rustqual.toml`:
```toml
exclude_files = [
    "src/adapters/mocks.rs",
    "tests/*",
]

[complexity]
detect_error_handling = true
```

`crates/minibox/rustqual.toml` — update `exclude_files`:
```toml
exclude_files = [
    "tests/*",
    "benches/*",
    "src/adapters/mocks.rs",
    "src/testing/*",
    "examples/*",
]
```

`crates/minibox-testsuite/rustqual.toml` (new):
```toml
# Test suite crate — all code is test infrastructure.
# ERROR_HANDLING (.unwrap/.expect) and MAGIC_NUMBER findings are
# expected and acceptable in test assertions and setup code.
ignore_functions = ["main", "test_*"]

[complexity]
detect_error_handling = false
detect_magic_numbers = false

[duplicates]
detect_dead_code = false
```

1. Apply config changes to all three `rustqual.toml` files.
2. Verify:
   ```
   rustqual crates/minibox-core/ --no-fail 2>&1 | grep ERROR_HANDLING  -> 0 from mocks
   rustqual crates/minibox/ --no-fail 2>&1 | grep ERROR_HANDLING  -> 0 from mocks/testing
   rustqual crates/minibox-testsuite/ --no-fail 2>&1 | grep ERROR_HANDLING  -> 0
   ```
3. Commit: `chore: exclude mock/test files from rustqual error-handling checks`

### Task 5: Extract test macro in minibox-crux-plugin

**Crate**: `minibox-crux-plugin`
**File(s)**: `crates/minibox-crux-plugin/tests/integration.rs`
**Run**: `cargo nextest run -p minibox-crux-plugin`

The 14 DUPLICATE findings share two patterns:

**Pattern A** — "invoke handler, assert InvokeOk, shutdown" (8 tests):
```rust
let tmp = TempDir::new().expect("tempdir");
let (listener, socket_path) = bind_mock(&tmp);
tokio::spawn(mock_daemon_once(listener, $response));
let mut h = PluginHarness::spawn(&socket_path);
let resp = h.invoke($handler, $input).await;
assert_eq!(resp["status"], "InvokeOk");
h.shutdown().await;
```

**Pattern B** — "invoke, assert InvokeOk, capture request, assert
match, shutdown" (4 tests):
```rust
let tmp = TempDir::new().expect("tempdir");
let (listener, socket_path) = bind_mock(&tmp);
let (tx, rx) = oneshot::channel();
tokio::spawn(mock_daemon_verify(listener, $response, tx));
let mut h = PluginHarness::spawn(&socket_path);
let resp = h.invoke($handler, $input).await;
assert_eq!(resp["status"], "InvokeOk");
let req = rx.await.expect("request captured");
$assert_req
h.shutdown().await;
```

Extract two macros at the top of the test file:

```rust
/// Invoke a handler against a mock daemon returning `$response`,
/// assert the plugin returns InvokeOk.
macro_rules! assert_invoke_ok {
    ($handler:expr, $input:expr, $response:expr) => {{
        let tmp = TempDir::new().expect("tempdir");
        let (listener, socket_path) = bind_mock(&tmp);
        tokio::spawn(mock_daemon_once(listener, $response));
        let mut h = PluginHarness::spawn(&socket_path);

        let resp = h.invoke($handler, $input).await;
        assert_eq!(resp["status"], "InvokeOk");

        h.shutdown().await;
    }};
}

/// Invoke a handler, assert InvokeOk, capture the daemon request,
/// and run `$assert` with the captured `DaemonRequest`.
macro_rules! assert_invoke_ok_and_verify {
    ($handler:expr, $input:expr, $response:expr, |$req:ident| $assert:expr) => {{
        let tmp = TempDir::new().expect("tempdir");
        let (listener, socket_path) = bind_mock(&tmp);
        let (tx, rx) = oneshot::channel();
        tokio::spawn(mock_daemon_verify(listener, $response, tx));
        let mut h = PluginHarness::spawn(&socket_path);

        let resp = h.invoke($handler, $input).await;
        assert_eq!(resp["status"], "InvokeOk");

        let $req = rx.await.expect("request captured");
        $assert

        h.shutdown().await;
    }};
}
```

Then rewrite each Pattern A test. Example — `invoke_pull_returns_success`:

```rust
#[tokio::test]
async fn invoke_pull_returns_success() {
    assert_invoke_ok!(
        "minibox::image::pull",
        json!({"image": "alpine"}),
        DaemonResponse::Success { message: "pulled".into() }
    );
}
```

And each Pattern B test. Example — `invoke_ps_sends_list_request`:

```rust
#[tokio::test]
async fn invoke_ps_sends_list_request() {
    assert_invoke_ok_and_verify!(
        "minibox::container::ps",
        json!({}),
        DaemonResponse::ContainerList { containers: vec![] },
        |req| {
            assert!(
                matches!(req, DaemonRequest::List),
                "expected List, got: {req:?}"
            );
        }
    );
}
```

Apply to all 14 duplicate tests. Leave streaming tests
(`invoke_exec_returns_streaming_output`, `invoke_build_returns_streaming_output`,
`invoke_logs_returns_streaming_output`) untouched — they use
`mock_daemon_multi` with different assertion shapes.

1. Add the two macros after the `PluginHarness` impl block.
2. Rewrite the 8 Pattern A tests and 4 Pattern B tests to use macros.
3. Verify:
   ```
   cargo nextest run -p minibox-crux-plugin    -> all green
   rustqual crates/minibox-crux-plugin/ --no-fail 2>&1 | grep DUPLICATE  -> 0
   ```
4. Commit: `refactor(minibox-crux-plugin): extract test macros to eliminate duplicate patterns`

### Task 6: Final verification

**Run**: `rustqual crates/minibox-core/ --no-fail; rustqual crates/minibox/ --no-fail; rustqual crates/miniboxd/ --no-fail; rustqual crates/mbx/ --no-fail; rustqual crates/minibox-crux-plugin/ --no-fail`

Expected score improvements:

| Crate | Before | Target |
|---|---|---|
| minibox-core | 85.1% | ~91% |
| minibox | 82.2% | ~86% |
| miniboxd | 93.0% | ~96% |
| mbx | 83.8% | ~88% |
| minibox-crux-plugin | 79.1% | ~95% |

Commit: none (verification only). If any test failures, return to the
relevant task to fix.
