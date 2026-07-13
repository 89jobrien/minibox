#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::doc_markdown,
    clippy::redundant_field_names,
    clippy::uninlined_format_args,
    clippy::redundant_clone,
    clippy::redundant_closure,
    clippy::single_char_pattern,
    clippy::unwrap_in_result,
    clippy::collapsible_if,
    clippy::match_same_arms,
    clippy::only_used_in_recursion,
    clippy::used_underscore_binding,
    clippy::map_unwrap_or,
    clippy::manual_assert,
    clippy::as_ptr_cast_mut,
    clippy::ptr_as_ptr,
    clippy::must_use_candidate,
    clippy::used_underscore_items,
    clippy::missing_const_for_fn,
    clippy::manual_string_new,
    clippy::semicolon_if_nothing_returned,
    clippy::unreadable_literal,
    clippy::default_constructed_unit_structs,
    clippy::ref_as_ptr,
    clippy::allow_attributes_without_reason,
    clippy::redundant_closure_for_method_calls,
    clippy::needless_raw_string_hashes,
    clippy::manual_is_variant_and,
    clippy::ignore_without_reason,
    clippy::default_trait_access,
    clippy::cast_lossless,
    clippy::match_wild_err_arm,
    clippy::format_push_string,
    clippy::bool_assert_comparison,
    clippy::struct_excessive_bools,
    clippy::type_complexity,
    clippy::float_cmp,
    clippy::diverging_sub_expression,
    clippy::single_match_else
)]
//! Unit tests for slashcrux vocabulary type integrations:
//! - min_priority gate
//! - StepState From<StepStatus> impl
//! - ExecutionContext env injection

use minibox_core::domain::{ExecutionContext, Priority, StepState, StepStatus};

// ---------------------------------------------------------------------------
// min_priority gate
// ---------------------------------------------------------------------------

#[test]
fn min_priority_gate_passes_when_at_threshold() {
    assert!(minibox_core::domain::meets_min_priority(
        &Priority::High,
        &Priority::High,
    ));
}

#[test]
fn min_priority_gate_passes_when_above_threshold() {
    assert!(minibox_core::domain::meets_min_priority(
        &Priority::Critical,
        &Priority::High,
    ));
}

#[test]
fn min_priority_gate_rejects_below_threshold() {
    assert!(!minibox_core::domain::meets_min_priority(
        &Priority::Low,
        &Priority::High,
    ));
}

#[test]
fn min_priority_gate_deferred_rejected_by_medium() {
    assert!(!minibox_core::domain::meets_min_priority(
        &Priority::Deferred,
        &Priority::Medium,
    ));
}

#[test]
fn min_priority_gate_medium_passes_medium() {
    assert!(minibox_core::domain::meets_min_priority(
        &Priority::Medium,
        &Priority::Medium,
    ));
}

// ---------------------------------------------------------------------------
// StepState From<StepStatus>
// ---------------------------------------------------------------------------

#[test]
fn step_state_from_pending() {
    let state: StepState = StepStatus::Pending.into();
    assert_eq!(state, StepState::Pending);
}

#[test]
fn step_state_from_running() {
    let state: StepState = StepStatus::Running.into();
    assert_eq!(state, StepState::Running);
}

#[test]
fn step_state_from_succeeded() {
    let state: StepState = StepStatus::Succeeded.into();
    assert_eq!(state, StepState::Completed);
}

#[test]
fn step_state_from_failed() {
    let state: StepState = StepStatus::Failed.into();
    assert_eq!(state, StepState::Failed);
}

#[test]
fn step_state_from_skipped() {
    let state: StepState = StepStatus::Skipped.into();
    assert_eq!(state, StepState::Skipped);
}

#[test]
fn step_state_from_errored() {
    let state: StepState = StepStatus::Errored.into();
    assert_eq!(state, StepState::Failed);
}

// ---------------------------------------------------------------------------
// ExecutionContext env injection
// ---------------------------------------------------------------------------

#[test]
fn execution_context_to_env_empty() {
    let ctx = ExecutionContext::new();
    let env = minibox_core::protocol::execution_context_to_env(&ctx);
    assert!(env.is_empty());
}

#[test]
fn execution_context_to_env_string_values() {
    let mut ctx = ExecutionContext::new();
    ctx.set("FOO", serde_json::Value::String("bar".into()));
    ctx.set("BAZ", serde_json::Value::String("qux".into()));
    let env = minibox_core::protocol::execution_context_to_env(&ctx);
    assert!(env.contains(&"FOO=bar".to_string()));
    assert!(env.contains(&"BAZ=qux".to_string()));
}

#[test]
fn execution_context_to_env_number_values() {
    let mut ctx = ExecutionContext::new();
    ctx.set("PORT", serde_json::Value::Number(8080.into()));
    let env = minibox_core::protocol::execution_context_to_env(&ctx);
    assert!(env.contains(&"PORT=8080".to_string()));
}

#[test]
fn execution_context_to_env_bool_values() {
    let mut ctx = ExecutionContext::new();
    ctx.set("DEBUG", serde_json::Value::Bool(true));
    let env = minibox_core::protocol::execution_context_to_env(&ctx);
    assert!(env.contains(&"DEBUG=true".to_string()));
}

#[test]
fn execution_context_to_env_null_skipped() {
    let mut ctx = ExecutionContext::new();
    ctx.set("PRESENT", serde_json::Value::String("yes".into()));
    ctx.set("NULL_VAL", serde_json::Value::Null);
    let env = minibox_core::protocol::execution_context_to_env(&ctx);
    assert_eq!(env.len(), 1);
    assert!(env.contains(&"PRESENT=yes".to_string()));
}

#[test]
fn execution_context_to_env_unset_skipped() {
    let mut ctx = ExecutionContext::new();
    ctx.set("KEEP", serde_json::Value::String("yes".into()));
    ctx.unset("GONE");
    let env = minibox_core::protocol::execution_context_to_env(&ctx);
    assert_eq!(env.len(), 1);
    assert!(env.contains(&"KEEP=yes".to_string()));
}

#[test]
fn execution_context_to_env_complex_json_stringified() {
    let mut ctx = ExecutionContext::new();
    ctx.set("DATA", serde_json::json!({"key": "value"}));
    let env = minibox_core::protocol::execution_context_to_env(&ctx);
    assert_eq!(env.len(), 1);
    // Complex values get JSON-stringified
    let val = &env[0];
    assert!(val.starts_with("DATA="));
    assert!(val.contains("key"));
}
