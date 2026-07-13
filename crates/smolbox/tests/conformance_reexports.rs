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
    clippy::duration_suboptimal_units,
    clippy::incompatible_msrv,
    clippy::suspicious_map,
    clippy::unnecessary_map_or,
    clippy::let_unit_value,
    clippy::ignored_unit_patterns
)]

//! Conformance: verify smolbox re-exports are accessible and
//! the adapter types satisfy expected trait bounds.

#[test]
fn smolvm_runtime_is_accessible() {
    fn _assert_send<T: Send>() {}
    _assert_send::<smolbox::smolvm::SmolVmRuntime>();
}

#[test]
fn krun_runtime_is_accessible() {
    fn _assert_send<T: Send>() {}
    _assert_send::<smolbox::krun::KrunRuntime>();
}

#[test]
fn preflight_check_smolvm_returns_status() {
    let status = smolbox::preflight::check_smolvm();
    assert_eq!(status.found, status.path.is_some());
}
