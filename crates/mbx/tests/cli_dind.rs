//! End-to-end `--dind` test: spawns a real `miniboxd`, runs `mbx run --dind`
//! against it, and asserts that the nested daemon actually started inside
//! the container (not a mocked daemon response — this exercises the real
//! namespace/cgroup/overlay stack plus the `--dind` entrypoint wrapper).
//!
//! Ignored by default: requires Linux, root, a working adapter (native,
//! smolvm, or krun), and network access to pull the base image. Run with:
//!   cargo nextest run -p minibox-cli --test `cli_dind` -- --include-ignored
#![cfg(all(unix, feature = "subprocess-tests"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use assert_cmd::Command;
use std::time::Duration;
use tempfile::TempDir;

mod test_helpers;
use test_helpers::wait_for_socket;

/// Locate a workspace binary by name.
///
/// Search order:
/// 1. `MINIBOX_TEST_BIN_DIR` env var (set by `just test-cli-subprocess`)
/// 2. `target/release/<name>`
/// 3. `target/debug/<name>`
fn find_binary(name: &str) -> std::path::PathBuf {
    if let Ok(dir) = std::env::var("MINIBOX_TEST_BIN_DIR") {
        let p = std::path::PathBuf::from(&dir).join(name);
        if p.exists() {
            return p;
        }
    }
    let manifest_dir = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("could not find workspace root");
    for profile in ["release", "debug"] {
        let p = workspace_root.join("target").join(profile).join(name);
        if p.exists() {
            return p;
        }
    }
    panic!(
        "Could not find {name} binary. Run `cargo build -p miniboxd -p minibox-cli` first, \
         or set MINIBOX_TEST_BIN_DIR."
    );
}

/// The daemon stages `--dind` resources by locating the host `miniboxd` via
/// `which miniboxd` or a handful of standard install paths (see
/// `crates/minibox/src/daemon/handler/run/dind.rs::find_miniboxd_binary`,
/// which runs inside the daemon process, not the `mbx` client). Neither
/// finds a freshly built `target/{debug,release}/miniboxd`, so prepend its
/// directory to `PATH` for the `miniboxd` subprocess.
fn path_with_miniboxd_dir(miniboxd: &std::path::Path) -> std::ffi::OsString {
    let dir = miniboxd
        .parent()
        .expect("miniboxd path should have a parent directory");
    let existing = std::env::var_os("PATH").unwrap_or_default();
    let mut joined = std::ffi::OsString::from(dir);
    joined.push(":");
    joined.push(existing);
    joined
}

#[tokio::test]
#[ignore = "requires Linux, root, a working adapter, and network access to pull alpine"]
async fn dind_run_starts_nested_daemon() {
    let tmp = TempDir::new().unwrap();
    let socket_path = tmp.path().join("outer.sock");
    let data_dir = tmp.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let miniboxd_bin = find_binary("miniboxd");
    let mbx_bin = find_binary("mbx");
    let path = path_with_miniboxd_dir(&miniboxd_bin);

    // The daemon process is what calls `find_miniboxd_binary` to stage the
    // nested `--dind` daemon, so it needs miniboxd's directory on its own
    // PATH — not the `mbx` client's.
    let mut daemon = std::process::Command::new(&miniboxd_bin)
        .env("MINIBOX_SOCKET_PATH", &socket_path)
        .env("MINIBOX_DATA_DIR", &data_dir)
        .env("PATH", &path)
        .spawn()
        .expect("failed to spawn miniboxd");

    wait_for_socket(&socket_path, 5000).await;

    let assert_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Command::from_std(std::process::Command::new(&mbx_bin))
            .env("MINIBOX_SOCKET_PATH", &socket_path)
            .args(["run", "--dind", "--rm", "alpine", "true"])
            .timeout(Duration::from_secs(60))
            .assert()
            .success();
    }));

    let _ = daemon.kill();
    let _ = daemon.wait();

    assert_result.expect("mbx run --dind should exit 0 once the nested daemon's socket is up");
}
