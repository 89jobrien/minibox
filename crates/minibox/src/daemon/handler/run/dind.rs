//! Daemon-side `DinD` resource staging.
//!
//! Ported from the client-side `crates/mbx/src/commands/run.rs` implementation
//! so the staging directory's lifetime is tied to the container record (daemon
//! state), not the `mbx` client process. The staging dir is persistent
//! (`/var/lib/minibox/dind-<container-id>/`), not a `tempfile::TempDir`, so it
//! survives independently of any client connection and is cleaned up by the
//! remove handler when the container is removed.
//!
//! Staging mechanics ([`dind_provision`]) are kept separate from host
//! discovery/preflight checks ([`stage_dind_resources`]) so unit tests can
//! exercise the mount/env layout with dummy files and a scratch root, without
//! requiring a real Linux host with cgroup v2 mounted — the same split the
//! retired client-side code used (`dind_provision` vs `stage_dind`).
//!
//! `stage_dind_resources` is not called yet — Task 3 of
//! `.ctx/godmode/plans/2026-10-27-dind-daemon-ownership.md` wires it into
//! `prepare_run`. The module-level dead-code allow below is removed in that
//! task once the public entry point has a real caller.

#![allow(dead_code)]

use anyhow::{Context as _, Result};
use minibox_core::domain::BindMount;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// Directory the `DinD` helper files are mounted at inside the container.
const DIND_DIR_PATH: &str = "/usr/local/lib/mbx-dind";

/// Path the miniboxd binary is installed to inside the container.
const DIND_MINIBOXD_PATH: &str = "/usr/local/lib/mbx-dind/miniboxd";

/// Path the dind entrypoint wrapper script is installed to inside the
/// container.
const DIND_WRAPPER_PATH: &str = "/usr/local/lib/mbx-dind/mbx-dind-entrypoint";

/// Path the nested `mbx` CLI is installed to inside the container.
const DIND_MBX_PATH: &str = "/usr/local/bin/mbx";

/// Directory the nested daemon keeps its state in.
///
/// This must be a bind mount from the host rather than a path on the outer
/// container's overlay: the nested daemon creates its own overlay mounts, and
/// the kernel does not permit overlay-on-overlay. Backing it with a host dir
/// is what makes nested runs work.
const DIND_DATA_DIR_PATH: &str = "/minibox-data";

/// Directory the nested daemon's socket lives in.
const DIND_RUN_DIR_PATH: &str = "/minibox-run";

/// Socket the nested daemon listens on, and the value injected as
/// `MINIBOX_SOCKET_PATH` so a nested `mbx` connects to it instead of the outer
/// daemon.
const DIND_SOCKET_PATH: &str = "/minibox-run/miniboxd.sock";

/// Parent cgroup filesystem, bind-mounted so the nested daemon can create its
/// own delegated slice.
const DIND_CGROUP_FS_PATH: &str = "/sys/fs/cgroup";

/// Persistent host-side root directory under which each container's `DinD`
/// staging directory lives.
const DIND_STAGING_ROOT: &str = "/var/lib/minibox";

/// Host-side staging plus the mounts/env a `--dind` run needs.
///
/// Unlike the retired client-side `DindProvisioning`, `staging_dir` is a
/// persistent path keyed by container ID, not a `tempfile::TempDir` — its
/// lifetime is tied to the container record (removed explicitly by the
/// remove handler), not to any process holding it alive.
pub(super) struct DindProvisioning {
    /// Host-side staging directory; persists until the container is removed.
    pub staging_dir: PathBuf,
    /// Extra bind mounts for the run request.
    pub mounts: Vec<BindMount>,
    /// Environment the wrapper and any nested `mbx` rely on.
    pub env: Vec<String>,
    /// Entrypoint override pointing at the staged wrapper script.
    pub entrypoint: String,
}

/// Persistent staging directory for a given container's `DinD` resources.
fn dind_staging_dir(container_id: &str) -> PathBuf {
    PathBuf::from(format!("{DIND_STAGING_ROOT}/dind-{container_id}"))
}

/// Locate the `miniboxd` binary that will run inside the container.
///
/// The daemon process itself is `miniboxd`, so this resolves to its own
/// executable rather than searching `PATH` the way the client-side version
/// had to.
fn find_miniboxd_binary() -> Result<PathBuf> {
    std::env::current_exe().context("resolve current miniboxd executable for DinD")
}

/// Locate the nested `mbx` CLI binary to stage alongside the daemon.
///
/// Distinct from `miniboxd`: the daemon process is not the `mbx` binary, so
/// this cannot use `current_exe()` the way `find_miniboxd_binary` does. Looks
/// on `PATH`, then common install locations, mirroring the daemon's other
/// binary-discovery fallbacks.
fn find_mbx_binary() -> Result<PathBuf> {
    if let Some(path_var) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path_var) {
            let candidate = dir.join("mbx");
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    for candidate in ["/usr/local/bin/mbx", "/usr/bin/mbx"] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }
    anyhow::bail!("mbx binary not found on PATH; required to stage nested CLI for --dind")
}

/// Validate the staged miniboxd binary is executable.
///
/// The daemon only runs on Linux (`DinD` additionally requires a mounted
/// cgroup v2 filesystem, checked separately), so unlike the retired
/// client-side check this does not need to detect a cross-platform (Mach-O)
/// binary — only the permission bit is load-bearing here.
fn validate_dind_binary(miniboxd: &Path) -> Result<()> {
    let metadata = std::fs::metadata(miniboxd)
        .with_context(|| format!("failed to stat miniboxd at {}", miniboxd.display()))?;
    if metadata.permissions().mode() & 0o111 == 0 {
        anyhow::bail!(
            "miniboxd at {} is not executable (mode: {:o})",
            miniboxd.display(),
            metadata.permissions().mode()
        );
    }
    Ok(())
}

/// Stage the `DinD` helper files under `root` and describe the mounts/env a
/// `--dind` run needs.
///
/// Pure staging mechanics: takes already-resolved host binary paths and a
/// destination root, so it can be exercised in unit tests with dummy files
/// and a scratch directory, independent of [`stage_dind_resources`]'s
/// Linux/cgroup-v2 preflight checks.
///
/// Layout of the staging directory (all mounted into the container):
///
/// * `miniboxd` — nested daemon binary, mounted at [`DIND_MINIBOXD_PATH`]
/// * `mbx-dind-entrypoint` — wrapper script, mounted at [`DIND_WRAPPER_PATH`]
/// * `mbx` — nested CLI, mounted at [`DIND_MBX_PATH`]
/// * `data/`, `run/` — nested daemon state, mounted at the container-side
///   data/run dirs so it never touches the outer container's overlay
fn dind_provision(
    container_id: &str,
    root: &Path,
    miniboxd_host: &Path,
    mbx_host: &Path,
    dind_debug: bool,
) -> Result<DindProvisioning> {
    std::fs::create_dir_all(root).context("create DinD staging directory")?;

    let miniboxd_dest = root.join("miniboxd");
    std::fs::copy(miniboxd_host, &miniboxd_dest)
        .with_context(|| format!("copy miniboxd from {}", miniboxd_host.display()))?;
    std::fs::set_permissions(&miniboxd_dest, std::fs::Permissions::from_mode(0o755))
        .context("make staged miniboxd executable")?;

    let wrapper_dest = root.join("mbx-dind-entrypoint");
    std::fs::write(
        &wrapper_dest,
        dind_entrypoint_script_for_container(dind_debug),
    )
    .context("write DinD entrypoint wrapper")?;
    std::fs::set_permissions(&wrapper_dest, std::fs::Permissions::from_mode(0o755))
        .context("make DinD entrypoint wrapper executable")?;

    let mbx_dest = root.join("mbx");
    std::fs::copy(mbx_host, &mbx_dest)
        .with_context(|| format!("copy mbx from {}", mbx_host.display()))?;
    std::fs::set_permissions(&mbx_dest, std::fs::Permissions::from_mode(0o755))
        .context("make staged mbx executable")?;

    let data_dir = root.join("data");
    let run_dir = root.join("run");
    std::fs::create_dir_all(&data_dir).context("create nested daemon data directory")?;
    std::fs::create_dir_all(&run_dir).context("create nested daemon run directory")?;

    let mounts = vec![
        BindMount {
            host_path: root.to_path_buf(),
            container_path: PathBuf::from(DIND_DIR_PATH),
            read_only: true,
        },
        BindMount {
            host_path: mbx_dest,
            container_path: PathBuf::from(DIND_MBX_PATH),
            read_only: true,
        },
        BindMount {
            host_path: data_dir,
            container_path: PathBuf::from(DIND_DATA_DIR_PATH),
            read_only: false,
        },
        BindMount {
            host_path: run_dir,
            container_path: PathBuf::from(DIND_RUN_DIR_PATH),
            read_only: false,
        },
        BindMount {
            host_path: PathBuf::from(DIND_CGROUP_FS_PATH),
            container_path: PathBuf::from(DIND_CGROUP_FS_PATH),
            read_only: false,
        },
    ];

    // NOTE: the outer image cache is deliberately *not* mounted into the
    // nested daemon — see crates/mbx/src/commands/run.rs's historical
    // dind_provision for the rationale (bind-mount-inside-bind-mount
    // path-traversal rejection); unchanged by moving this to the daemon.

    let cgroup_root = format!("{DIND_CGROUP_FS_PATH}/minibox-dind-{container_id}");

    Ok(DindProvisioning {
        staging_dir: root.to_path_buf(),
        mounts,
        env: vec![
            format!("MINIBOX_DATA_DIR={DIND_DATA_DIR_PATH}"),
            format!("MINIBOX_RUN_DIR={DIND_RUN_DIR_PATH}"),
            format!("MINIBOX_SOCKET_PATH={DIND_SOCKET_PATH}"),
            format!("MINIBOX_CGROUP_ROOT={cgroup_root}"),
        ],
        entrypoint: DIND_WRAPPER_PATH.to_string(),
    })
}

/// Resolve everything a `--dind` run needs before the container is spawned.
///
/// Entry point called from `prepare_run` when `RunParams.dind` is true. Runs
/// the Linux/cgroup-v2 preflight checks and host binary discovery that
/// [`dind_provision`] deliberately omits, then stages under the container's
/// persistent directory.
pub(super) fn stage_dind_resources(
    container_id: &str,
    dind_debug: bool,
) -> Result<DindProvisioning> {
    let miniboxd_host = find_miniboxd_binary()?;
    validate_dind_binary(&miniboxd_host).context("DinD binary validation failed")?;
    if !Path::new(DIND_CGROUP_FS_PATH).is_dir() {
        anyhow::bail!(
            "--dind requires a Linux host with a mounted cgroup v2 filesystem at {DIND_CGROUP_FS_PATH}"
        );
    }
    let mbx_host = find_mbx_binary()?;
    dind_provision(
        container_id,
        &dind_staging_dir(container_id),
        &miniboxd_host,
        &mbx_host,
        dind_debug,
    )
}

/// Entrypoint wrapper script with health monitoring: starts a nested daemon,
/// monitors its health, times out on socket poll, and fails fast on startup
/// errors. When `dind_debug` is true, captures miniboxd stderr for
/// diagnostics.
///
/// The nested daemon's data/run dirs, socket path, and cgroup root all come
/// from environment variables staged by [`dind_provision`]; nothing here is
/// derived from the outer daemon's own paths.
fn dind_entrypoint_script_for_container(dind_debug: bool) -> String {
    let miniboxd_path = DIND_MINIBOXD_PATH;
    let sock = DIND_SOCKET_PATH;
    let debug_stderr_redirect = if dind_debug {
        " 2>/tmp/miniboxd-debug.log"
    } else {
        " 2>/dev/null"
    };
    format!(
        "#!/bin/sh\n\
         set -e\n\
         \n\
         echo \"[dind] version: host=$MINIBOX_CLIENT_VERSION, nested=$({miniboxd_path} --version 2>/dev/null || echo unknown)\" >&2\n\
         echo \"[dind] pre-flight validation starting\" >&2\n\
         \n\
         # Validate /bin/sh exists (fail gracefully on scratch images)\n\
         if [ ! -f /bin/sh ]; then\n\
             echo \"ERROR: /bin/sh does not exist in container (scratch image?)\" >&2\n\
             exit 124\n\
         fi\n\
         \n\
         # Validate miniboxd executable exists and has execute permission\n\
         if [ ! -x \"{miniboxd_path}\" ]; then\n\
             if [ ! -f \"{miniboxd_path}\" ]; then\n\
                 echo \"ERROR: miniboxd binary not found at {miniboxd_path}\" >&2\n\
                 exit 125\n\
             else\n\
                 echo \"ERROR: miniboxd at {miniboxd_path} is not executable\" >&2\n\
                 exit 125\n\
             fi\n\
         fi\n\
         \n\
         # Give the nested daemon its own delegated cgroup slice. The outer\n\
         # cgroup filesystem is bind-mounted, so the slice name is unique per\n\
         # run and parallel DinD containers do not collide.\n\
         if [ -n \"$MINIBOX_CGROUP_ROOT\" ]; then\n\
             mkdir -p \"$MINIBOX_CGROUP_ROOT\" 2>/dev/null || true\n\
             if [ -f \"$MINIBOX_CGROUP_ROOT/cgroup.subtree_control\" ]; then\n\
                 echo '+memory +cpu +pids' > \"$MINIBOX_CGROUP_ROOT/cgroup.subtree_control\" 2>/dev/null || true\n\
             fi\n\
         fi\n\
         \n\
         echo \"[dind] starting nested miniboxd from {miniboxd_path}\" >&2\n\
         {miniboxd_path}{debug_stderr_redirect} & DAEMON_PID=$!\n\
         \n\
         # Monitor daemon and exit if it dies\n\
         if ! kill -0 $DAEMON_PID 2>/dev/null; then\n\
             echo \"ERROR: nested miniboxd failed to start (PID $DAEMON_PID)\" >&2\n\
             if [ -f /tmp/miniboxd-debug.log ]; then\n\
                 echo \"[dind] daemon stderr:\" >&2\n\
                 cat /tmp/miniboxd-debug.log >&2\n\
             fi\n\
             exit 126\n\
         fi\n\
         \n\
         echo \"[dind] daemon started (PID $DAEMON_PID), waiting for socket\" >&2\n\
         \n\
         # Poll socket with timeout (10s max)\n\
         TIMEOUT=100\n\
         COUNT=0\n\
         while [ ! -S {sock} ] && [ $COUNT -lt $TIMEOUT ]; do\n\
             sleep 0.1\n\
             COUNT=$((COUNT + 1))\n\
         done\n\
         \n\
         if [ ! -S {sock} ]; then\n\
             echo \"ERROR: nested miniboxd socket never appeared after 10s\" >&2\n\
             if [ -f /tmp/miniboxd-debug.log ]; then\n\
                 echo \"[dind] daemon stderr:\" >&2\n\
                 cat /tmp/miniboxd-debug.log >&2\n\
             fi\n\
             echo \"[dind] attempting to diagnose: killing daemon (PID $DAEMON_PID)\" >&2\n\
             kill $DAEMON_PID 2>/dev/null || true\n\
             wait $DAEMON_PID 2>/dev/null || true\n\
             exit 127\n\
         fi\n\
         \n\
         # Point the container's own `mbx` at the nested daemon, then hand off\n\
         # to the original command.\n\
         export MINIBOX_SOCKET_PATH={sock}\n\
         echo \"[dind] socket ready at {sock}, executing original command\" >&2\n\
         exec \"$@\"\n"
    )
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;

    fn stage_with_dummy_binaries(container_id: &str) -> (tempfile::TempDir, DindProvisioning) {
        let src = tempfile::tempdir().expect("temp source dir");
        let miniboxd = src.path().join("miniboxd");
        let mbx = src.path().join("mbx");
        std::fs::write(&miniboxd, b"#!/bin/sh\nexit 0\n").expect("write dummy miniboxd");
        std::fs::set_permissions(&miniboxd, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::write(&mbx, b"#!/bin/sh\nexit 0\n").expect("write dummy mbx");
        std::fs::set_permissions(&mbx, std::fs::Permissions::from_mode(0o755)).unwrap();

        let root_holder = tempfile::tempdir().expect("temp staging root");
        let root = root_holder.path().join(format!("dind-{container_id}"));
        let provisioning = dind_provision(container_id, &root, &miniboxd, &mbx, false)
            .expect("stage dind resources");
        (root_holder, provisioning)
    }

    #[test]
    fn test_dind_provision_stages_wrapper_and_mounts() {
        let (_root_holder, provisioning) = stage_with_dummy_binaries("test-container-id");
        assert!(
            provisioning
                .mounts
                .iter()
                .any(|m| m.container_path.as_path() == Path::new(DIND_MBX_PATH)),
            "expected a mount for the nested mbx CLI"
        );
        assert_eq!(provisioning.entrypoint, DIND_WRAPPER_PATH);
    }

    #[test]
    fn test_dind_provision_targets_persistent_dir() {
        // The production entry point (stage_dind_resources) must compute a
        // persistent, container-id-keyed path, not a tempfile::TempDir.
        assert_eq!(
            dind_staging_dir("abc123"),
            PathBuf::from("/var/lib/minibox/dind-abc123"),
        );
    }
}
