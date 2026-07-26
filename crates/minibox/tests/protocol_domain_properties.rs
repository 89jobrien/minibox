//! Property-based tests for protocol codec edge cases and domain type validation.
//!
//! Complements proptest_suite.rs (which covers basic roundtrip invariants)
//! with targeted edge-case properties.

use minibox::domain::ContainerId;
use minibox::protocol::{
    DaemonRequest, DaemonResponse, OutputStreamKind, decode_request, decode_response,
    encode_request, encode_response,
};
use proptest::prelude::*;

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: None,
        ..ProptestConfig::default()
    })]

    // ── Protocol edge cases ───────────────────────────────────────────────────

    /// Embedded newlines inside JSON string fields must not corrupt framing.
    /// The JSON encoder escapes `\n` as `\\n`, so newlines in data are not
    /// message delimiters and the roundtrip must be lossless.
    #[test]
    fn embedded_newlines_dont_corrupt_framing(
        prefix in ".*",
        suffix in ".*",
    ) {
        let image = format!("{prefix}\n{suffix}");
        let req = DaemonRequest::Pull { image, tag: None, platform: None };
        let encoded = encode_request(&req).expect("unwrap in test");
        let decoded = decode_request(&encoded).expect("unwrap in test");
        let re_encoded = encode_request(&decoded).expect("unwrap in test");
        prop_assert_eq!(encoded, re_encoded);
    }

    /// Null bytes (`\0`) inside string fields must survive encode/decode.
    #[test]
    fn null_bytes_in_strings_survive_roundtrip(
        prefix in ".*",
        suffix in ".*",
    ) {
        let data = format!("{prefix}\x00{suffix}");
        let resp = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data,
        };
        let encoded = encode_response(&resp).expect("unwrap in test");
        let decoded = decode_response(&encoded).expect("unwrap in test");
        let re_encoded = encode_response(&decoded).expect("unwrap in test");
        prop_assert_eq!(encoded, re_encoded);
    }

    /// Strings up to 100 KB in output data fields must survive encode/decode
    /// without truncation or corruption.
    #[test]
    fn very_long_strings_roundtrip(
        data in proptest::collection::vec(any::<u8>(), 0..102_400)
            .prop_map(|bytes| String::from_utf8_lossy(&bytes).into_owned()),
    ) {
        let resp = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stderr,
            data,
        };
        let encoded = encode_response(&resp).expect("unwrap in test");
        let decoded = decode_response(&encoded).expect("unwrap in test");
        let re_encoded = encode_response(&decoded).expect("unwrap in test");
        prop_assert_eq!(encoded, re_encoded);
    }

    /// `DaemonRequest::Run` with an empty command vec must roundtrip correctly.
    #[test]
    fn empty_command_vec_roundtrips(image in any::<String>()) {
        let req = minibox_macros::test_run!(image: image, command: Vec::<String>::new());
        let encoded = encode_request(&req).expect("unwrap in test");
        let decoded = decode_request(&encoded).expect("unwrap in test");
        let re_encoded = encode_request(&decoded).expect("unwrap in test");
        prop_assert_eq!(encoded, re_encoded);
    }

    /// `u64::MAX` values for resource limit fields must survive encode/decode.
    #[test]
    fn max_u64_resource_values_roundtrip(image in any::<String>()) {
        let req = minibox_macros::test_run!(
            image: image,
            command: Vec::<String>::new(),
            memory_limit_bytes: Some(u64::MAX),
            cpu_weight: Some(u64::MAX),
        );
        let encoded = encode_request(&req).expect("unwrap in test");
        let decoded = decode_request(&encoded).expect("unwrap in test");
        let re_encoded = encode_request(&decoded).expect("unwrap in test");
        prop_assert_eq!(encoded, re_encoded);
    }

    // ── Domain type validation ────────────────────────────────────────────────

    /// `ContainerId::new("")` must always fail — empty IDs are never valid.
    #[test]
    fn container_id_rejects_empty_string(_dummy in Just(())) {
        prop_assert!(ContainerId::new(String::new()).is_err());
    }

    /// Any string longer than 64 characters must be rejected by `ContainerId`.
    #[test]
    fn container_id_rejects_strings_over_64_chars(
        // 65..=200 chars, all alphanumeric so the only rejection reason is length
        s in "[a-zA-Z0-9]{65,200}",
    ) {
        prop_assert!(
            ContainerId::new(s.clone()).is_err(),
            "expected Err for len={}, got Ok",
            s.len()
        );
    }

    /// Strings containing non-alphanumeric characters must be rejected.
    /// The non-alphanumeric char is injected at a random position inside a
    /// valid-length (1-64 char) string so length is never the rejection cause.
    #[test]
    fn container_id_rejects_non_alphanumeric_chars(
        prefix in "[a-zA-Z0-9]{0,30}",
        bad_char in prop_oneof![
            Just(' '), Just('-'), Just('_'), Just('.'), Just('/'),
            Just('!'), Just('@'), Just('#'), Just('$'), Just('%'),
        ],
        suffix in "[a-zA-Z0-9]{0,30}",
    ) {
        // Ensure total length stays within 64 so rejection is due to the char, not length.
        let mut s = format!("{prefix}{bad_char}{suffix}");
        s.truncate(64);
        // Verify the bad char survived truncation; if not, just skip the case.
        prop_assume!(s.chars().any(|c| !c.is_ascii_alphanumeric()));
        prop_assert!(
            ContainerId::new(s.clone()).is_err(),
            "expected Err for {:?}, got Ok",
            s
        );
    }

    /// Valid alphanumeric strings of length 1-64 must always be accepted.
    #[test]
    fn container_id_accepts_valid_alphanumeric_strings(
        s in "[a-zA-Z0-9]{1,64}",
    ) {
        prop_assert!(
            ContainerId::new(s.clone()).is_ok(),
            "expected Ok for {:?}, got Err",
            s
        );
    }
}
