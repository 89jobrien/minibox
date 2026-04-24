//! Security regression tests for daemonbox.
//!
//! These tests pin the socket auth (SO_PEERCRED UID check) invariants so that
//! any future refactor that weakens the gate is caught at compile time or in CI.
//!
//! No actual sockets or OS calls are needed — the auth predicate is pure logic.

use daemonbox::server::{PeerCreds, is_authorized};

mod security_regression {
    use super::*;

    // -------------------------------------------------------------------------
    // Socket auth — SO_PEERCRED UID gate
    // -------------------------------------------------------------------------

    #[test]
    fn root_uid_accepted_when_root_required() {
        let creds = PeerCreds { uid: 0, pid: 1234 };
        assert!(
            is_authorized(Some(&creds), true),
            "UID 0 must be accepted when require_root_auth=true"
        );
    }

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

    #[test]
    fn uid_1_rejected_when_root_required() {
        // UID 1 (daemon user) must not bypass the root gate.
        let creds = PeerCreds { uid: 1, pid: 9999 };
        assert!(
            !is_authorized(Some(&creds), true),
            "UID 1 must be rejected when require_root_auth=true"
        );
    }

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

    #[test]
    fn missing_creds_accepted_when_root_not_required() {
        // Platforms without SO_PEERCRED (e.g. Windows named pipes) pass None.
        // When root auth is disabled, this is allowed through.
        assert!(
            is_authorized(None, false),
            "None creds must be accepted when require_root_auth=false"
        );
    }

    #[test]
    fn missing_creds_still_allowed_through_when_root_required() {
        // When creds are unavailable (None) but root auth is required, the
        // server logs a warning and allows the connection through (bypassed).
        // This matches the current server.rs behaviour: the warn is emitted
        // but the connection proceeds. The test pins this behaviour so any
        // intentional change (e.g. reject on missing creds) must update here.
        assert!(
            is_authorized(None, true),
            "None creds bypass require_root_auth (logged as warning, not rejected)"
        );
    }

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
}
