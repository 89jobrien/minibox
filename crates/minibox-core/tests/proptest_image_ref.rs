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
    clippy::struct_excessive_bools
)]
//! Property-based tests for [`ImageRef::parse`].
//!
//! `ImageRef` is exercised indirectly via `proptest_suite::request_encode_decode_roundtrip`
//! (the `Pull` protocol variant carries arbitrary image/tag strings). These tests make the
//! parser's totality invariant explicit and independently verifiable: `parse` returns a
//! `Result` and must never panic, for any input whatsoever.

use minibox_core::image::reference::ImageRef;
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    /// `ImageRef::parse` never panics, for any arbitrary string input.
    #[test]
    fn image_ref_parse_never_panics(s in any::<String>()) {
        let _ = ImageRef::parse(&s);
    }

    /// Well-formed `name:tag` inputs always parse successfully.
    #[test]
    fn image_ref_roundtrip_valid_inputs(
        name in "[a-z][a-z0-9-]{0,20}",
        tag in "[a-z0-9][a-z0-9._-]{0,20}",
    ) {
        let input = format!("{name}:{tag}");
        let r = ImageRef::parse(&input);
        prop_assert!(r.is_ok(), "expected Ok for {input:?}, got {r:?}");
    }
}
