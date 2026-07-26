#!/usr/bin/env bash
set -euo pipefail

# Fast Kani proofs only -- skips slow parse_volume harnesses that trigger
# deep memchr/memcmp CBMC unwinding. Use `mise run kani` for the full suite.
#
# When adding new harnesses, add them here unless they are slow.

FAILED=0

echo "=== kani-quick: minibox-core ==="
if ! cargo kani --package minibox-core \
  --harness is_valid_env_key_single_byte_exhaustive \
  --harness is_valid_env_key_rejects_empty \
  --harness is_valid_env_key_rejects_injection_chars \
  --harness default_max_depth_is_three \
  --harness encode_response_appends_newline \
  --harness decode_strips_trailing_newline \
  --harness phase_outcome_ordering \
  --harness phase_outcome_total_order \
  --harness phase_outcome_max_is_worst \
  --harness step_status_to_state_total \
  --harness step_status_error_collapse \
  "$@"; then
  FAILED=1
  echo "FAIL: minibox-core (domain/protocol)"
fi

echo "=== kani-quick: minibox-core (execution_policy) ==="
if ! cargo kani --package minibox-core \
  --harness image_matches_wildcard_matches_all \
  --harness image_matches_exact_is_reflexive \
  --harness deny_before_allow_invariant \
  --harness memory_limit_denies_excess \
  --harness default_policy_always_allows \
  "$@"; then
  FAILED=1
  echo "FAIL: minibox-core (execution_policy)"
fi

echo "=== kani-quick: minibox-core (image/layer) ==="
if ! cargo kani --package minibox-core \
  --harness validate_tar_entry_path_rejects_dotdot \
  --harness has_parent_dir_component_equivalence \
  --harness relative_path_no_dotdot_when_inputs_clean \
  --harness setuid_mask_strips_special_bits \
  "$@"; then
  FAILED=1
  echo "FAIL: minibox-core (image/layer)"
fi

echo ""
echo "Skipped (slow -- use 'mise run kani' for full suite):"
echo "  - parse_volume_absolute_container_path"
echo "  - parse_volume_rejects_relative_container"

if [ "$FAILED" -eq 0 ]; then
  echo ""
  echo "All quick Kani proofs passed."
else
  echo ""
  echo "Some Kani proofs failed."
  exit 1
fi
