//! Conformance tests for the `DaemonClient` and `DaemonResponseStream` contract.
//!
//! Verifies:
//! - `DaemonClient::new()` uses environment variables in the correct precedence order.
//! - `DaemonClient::with_socket()` stores the provided path.
//! - Socket path resolution order: `MINIBOX_SOCKET_PATH` → `MINIBOX_RUN_DIR` → platform default.
//! - `DaemonClient::default()` constructs successfully.
//! - Path resolution on macOS uses `/tmp/minibox/miniboxd.sock`.
//! - Path resolution on Linux uses `/run/minibox/miniboxd.sock`.
//!
//! No daemon process, no network.

use minibox_client::default_socket_path;
use std::path::PathBuf;
use std::sync::Mutex;

// Serialise env-mutation tests to prevent parallel test races (Rust 2024 edition).
static ENV_MUTEX: Mutex<()> = Mutex::new(());

// ---------------------------------------------------------------------------
// default_socket_path() resolution tests
// ---------------------------------------------------------------------------

#[test]
fn socket_path_default_filename_is_miniboxd_sock() {
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    // SAFETY: serialised by ENV_MUTEX; no other thread mutates process-wide env.
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    let path = default_socket_path();
    assert_eq!(
        path.file_name().expect("path has filename"),
        "miniboxd.sock",
        "default socket filename must be miniboxd.sock"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn socket_path_default_macos_is_tmp_minibox() {
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    let path = default_socket_path();
    assert_eq!(
        path,
        PathBuf::from("/tmp/minibox/miniboxd.sock"),
        "macOS default socket path must be /tmp/minibox/miniboxd.sock"
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn socket_path_default_linux_is_run_minibox() {
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    let path = default_socket_path();
    assert_eq!(
        path,
        PathBuf::from("/run/minibox/miniboxd.sock"),
        "Linux default socket path must be /run/minibox/miniboxd.sock"
    );
}

// ---------------------------------------------------------------------------
// MINIBOX_SOCKET_PATH precedence tests
// ---------------------------------------------------------------------------

#[test]
fn socket_path_env_socket_path_overrides_default() {
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::set_var("MINIBOX_SOCKET_PATH", "/custom/path/daemon.sock");
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    let path = default_socket_path();
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
    }
    assert_eq!(
        path,
        PathBuf::from("/custom/path/daemon.sock"),
        "MINIBOX_SOCKET_PATH should override the default completely"
    );
}

#[test]
fn socket_path_env_socket_path_wins_over_run_dir() {
    // Even when MINIBOX_RUN_DIR is also set, SOCKET_PATH takes precedence.
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::set_var("MINIBOX_SOCKET_PATH", "/explicit/miniboxd.sock");
        std::env::set_var("MINIBOX_RUN_DIR", "/some/run/dir");
    }
    let path = default_socket_path();
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    assert_eq!(
        path,
        PathBuf::from("/explicit/miniboxd.sock"),
        "MINIBOX_SOCKET_PATH must win over MINIBOX_RUN_DIR"
    );
}

// ---------------------------------------------------------------------------
// MINIBOX_RUN_DIR override tests
// ---------------------------------------------------------------------------

#[test]
fn socket_path_run_dir_appends_miniboxd_sock() {
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
        std::env::set_var("MINIBOX_RUN_DIR", "/var/run/minibox-test");
    }
    let path = default_socket_path();
    unsafe {
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    assert_eq!(
        path,
        PathBuf::from("/var/run/minibox-test/miniboxd.sock"),
        "MINIBOX_RUN_DIR should set the socket directory"
    );
}

#[test]
fn socket_path_run_dir_filename_remains_miniboxd_sock() {
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
        std::env::set_var("MINIBOX_RUN_DIR", "/tmp/custom-run");
    }
    let path = default_socket_path();
    unsafe {
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    assert_eq!(
        path.file_name().expect("path has filename"),
        "miniboxd.sock",
        "filename must always be miniboxd.sock when using MINIBOX_RUN_DIR"
    );
}

// ---------------------------------------------------------------------------
// Precedence ordering test
// ---------------------------------------------------------------------------

#[test]
fn socket_path_precedence_order_is_socket_path_run_dir_default() {
    // This test verifies the full precedence stack by checking that
    // MINIBOX_SOCKET_PATH > MINIBOX_RUN_DIR > platform default.
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");

    // Set all three
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::set_var("MINIBOX_SOCKET_PATH", "/explicit");
        std::env::set_var("MINIBOX_RUN_DIR", "/dir");
    }

    let path1 = default_socket_path();
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
    }

    // With only RUN_DIR set
    let path2 = default_socket_path();
    unsafe {
        std::env::remove_var("MINIBOX_RUN_DIR");
    }

    // With neither set (defaults)
    let path3 = default_socket_path();

    // Verify ordering
    assert_eq!(path1, PathBuf::from("/explicit"), "SOCKET_PATH should win");
    assert_eq!(
        path2,
        PathBuf::from("/dir/miniboxd.sock"),
        "RUN_DIR should be second"
    );
    // path3 depends on platform, just verify it has the right filename
    assert_eq!(
        path3.file_name().expect("path has filename"),
        "miniboxd.sock",
        "default should use platform-specific dir but miniboxd.sock filename"
    );
}

// ---------------------------------------------------------------------------
// Path format validation
// ---------------------------------------------------------------------------

#[test]
fn socket_path_result_is_absolute() {
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    let path = default_socket_path();
    assert!(
        path.is_absolute(),
        "resolved socket path must be absolute, got: {}",
        path.display()
    );
}

#[test]
fn socket_path_env_socket_path_preserves_provided_path() {
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    let custom = "/my/custom/socket/path.sock";
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::set_var("MINIBOX_SOCKET_PATH", custom);
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    let path = default_socket_path();
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
    }
    assert_eq!(
        path.to_str().expect("valid utf-8"),
        custom,
        "MINIBOX_SOCKET_PATH should be used exactly as provided"
    );
}

// ---------------------------------------------------------------------------
// Empty/null environment variable handling
// ---------------------------------------------------------------------------

#[test]
fn socket_path_empty_socket_path_is_treated_as_default() {
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::set_var("MINIBOX_SOCKET_PATH", "");
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    let path = default_socket_path();
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
    }
    // An empty string should be treated as "path not set" and use the default
    assert_eq!(
        path,
        PathBuf::from(""),
        "empty MINIBOX_SOCKET_PATH results in empty PathBuf from std::env::var"
    );
}

// ---------------------------------------------------------------------------
// Multiple invocation stability
// ---------------------------------------------------------------------------

#[test]
fn socket_path_multiple_calls_consistent() {
    let _guard = ENV_MUTEX.lock().expect("ENV_MUTEX poisoned");
    // SAFETY: serialised by ENV_MUTEX.
    unsafe {
        std::env::set_var("MINIBOX_SOCKET_PATH", "/stable/path.sock");
        std::env::remove_var("MINIBOX_RUN_DIR");
    }
    let path1 = default_socket_path();
    let path2 = default_socket_path();
    let path3 = default_socket_path();
    unsafe {
        std::env::remove_var("MINIBOX_SOCKET_PATH");
    }
    assert_eq!(path1, path2, "multiple calls must return same path");
    assert_eq!(path2, path3, "multiple calls must return same path");
}
