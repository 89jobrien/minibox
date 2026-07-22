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
//! Security regression tests for the daemon handler.
//!
//! These tests pin the socket auth (SO_PEERCRED UID check) invariants so that
//! any future refactor that weakens the gate is caught at compile time or in CI.
//!
//! No actual sockets or OS calls are needed — the auth predicate is pure logic.

use minibox::daemon::server::{PeerCreds, is_authorized};

mod security_regression {
    use super::*;

    // -------------------------------------------------------------------------
    // Socket auth — SO_PEERCRED UID gate
    // Invariant: 7 — SO_PEERCRED Unix Socket Authentication
    // -------------------------------------------------------------------------

    // Invariant: 7 — SO_PEERCRED Unix Socket Authentication
    #[test]
    fn root_uid_accepted_when_root_required() {
        let creds = PeerCreds { uid: 0, pid: 1234 };
        assert!(
            is_authorized(Some(&creds), true),
            "UID 0 must be accepted when require_root_auth=true"
        );
    }

    // Invariant: 7 — SO_PEERCRED Unix Socket Authentication
    #[test]
    fn non_root_uid_rejected_when_root_required() {
        let creds = PeerCreds {
            uid: 1000,
            pid: 5678,
        };
        assert!(
            !is_authorized(Some(&creds), true),
            "non-root UID must be rejected when require_root_auth=true"
        );
    }

    // Invariant: 7 — SO_PEERCRED Unix Socket Authentication
    #[test]
    fn uid_1_rejected_when_root_required() {
        // UID 1 (daemon user) must not bypass the root gate.
        let creds = PeerCreds { uid: 1, pid: 9999 };
        assert!(
            !is_authorized(Some(&creds), true),
            "UID 1 must be rejected when require_root_auth=true"
        );
    }

    // Invariant: 7 — SO_PEERCRED Unix Socket Authentication
    #[test]
    fn any_uid_accepted_when_root_not_required() {
        let non_root = PeerCreds { uid: 1000, pid: 42 };
        assert!(
            is_authorized(Some(&non_root), false),
            "any UID must be accepted when require_root_auth=false"
        );

        let root = PeerCreds { uid: 0, pid: 1 };
        assert!(
            is_authorized(Some(&root), false),
            "UID 0 must be accepted when require_root_auth=false"
        );
    }

    // Invariant: 7 — SO_PEERCRED Unix Socket Authentication
    #[test]
    fn missing_creds_accepted_when_root_not_required() {
        // Platforms without SO_PEERCRED (e.g. Windows named pipes) pass None.
        // When root auth is disabled, this is allowed through.
        assert!(
            is_authorized(None, false),
            "None creds must be accepted when require_root_auth=false"
        );
    }

    // Invariant: 7 — SO_PEERCRED Unix Socket Authentication
    #[test]
    fn missing_creds_rejected_when_root_required() {
        // When creds are unavailable (None) and root auth is required, the
        // connection is rejected (fail-closed). This is the correct security
        // posture: unknown identity must not bypass the root gate.
        assert!(
            !is_authorized(None, true),
            "None creds must be rejected when require_root_auth=true (fail-closed)"
        );
    }

    // Invariant: 7 — SO_PEERCRED Unix Socket Authentication
    #[test]
    fn max_uid_rejected_when_root_required() {
        // Boundary check: u32::MAX must not be treated as root.
        let creds = PeerCreds {
            uid: u32::MAX,
            pid: 1,
        };
        assert!(
            !is_authorized(Some(&creds), true),
            "u32::MAX UID must be rejected when require_root_auth=true"
        );
    }

    // -------------------------------------------------------------------------
    // Exhaustive cross-product: (require_root: bool) x (uid: None, Some(...))
    // -------------------------------------------------------------------------

    // Invariant: 7 — SO_PEERCRED Unix Socket Authentication
    /// Enumerate ALL valid inputs for `is_authorized` over the full cross product
    /// of `require_root_auth` ∈ {false, true} and
    /// `uid` ∈ {None, Some(0), Some(1), Some(500), Some(65534), Some(u32::MAX)}.
    ///
    /// Expected outcomes follow the truth table in the function's doc comment:
    ///
    /// | require_root | creds          | expected |
    /// |--------------|----------------|----------|
    /// | false        | any / None     | true     |
    /// | true         | None           | false    |
    /// | true         | Some(uid == 0) | true     |
    /// | true         | Some(uid > 0)  | false    |
    #[test]
    fn exhaustive_is_authorized_cross_product() {
        // (uid_opt, require_root, expected_result)
        let cases: &[(Option<u32>, bool, bool)] = &[
            // require_root = false: everything is allowed regardless of uid
            (None, false, true),
            (Some(0), false, true),
            (Some(1), false, true),
            (Some(500), false, true),
            (Some(65534), false, true),
            (Some(u32::MAX), false, true),
            // require_root = true, no creds: denied (fail-closed)
            (None, true, false),
            // require_root = true, uid == 0: allowed
            (Some(0), true, true),
            // require_root = true, uid > 0: denied
            (Some(1), true, false),
            (Some(500), true, false),
            (Some(65534), true, false),
            (Some(u32::MAX), true, false),
        ];

        for &(uid_opt, require_root, expected) in cases {
            let creds = uid_opt.map(|uid| PeerCreds { uid, pid: 1 });
            let result = is_authorized(creds.as_ref(), require_root);
            assert_eq!(
                result, expected,
                "is_authorized(uid={uid_opt:?}, require_root={require_root}) \
                 expected {expected} but got {result}"
            );
        }
    }
}
