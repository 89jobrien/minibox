//! `minibox run` — create and start a container.
//!
//! Unlike the other command modules, this module uses streaming responses
//! via [`DaemonClient`] to handle output in real time. It sets `ephemeral: true`
//! on the request so the daemon streams output back instead of returning
//! immediately with a container ID.
//!
//! # Streaming protocol
//!
//! After the `Run` request is sent the daemon emits a sequence of messages:
//!
//! * [`DaemonResponse::ContainerOutput`] — a base64-encoded chunk of bytes
//!   from the container's stdout or stderr; forwarded verbatim to the
//!   corresponding local stream.
//! * [`DaemonResponse::ContainerStopped`] — signals that the container has
//!   exited; the CLI exits with the container's exit code.
//! * [`DaemonResponse::ContainerCreated`] — only sent by older daemon builds
//!   that do not support streaming; the CLI prints the container ID and exits
//!   with code 0.
//! * [`DaemonResponse::Error`] — a fatal error from the daemon; printed to
//!   stderr and the CLI exits with code 1.
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::doc_markdown,
        clippy::unwrap_in_result
    )
)]

use super::RequestError;
use anyhow::{Context as _, Result};
use base64::Engine;
use minibox_core::client::DaemonClient;
use minibox_core::domain::{BindMount, NetworkMode, UidRangeMode};
use minibox_core::protocol::{DaemonRequest, DaemonResponse, OutputStreamKind};
use std::io::{IsTerminal as _, Write};
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

/// Parse a `-v src:dst[:ro]` volume shorthand into a `BindMount`.
pub fn parse_volume(s: &str) -> anyhow::Result<BindMount> {
    BindMount::parse_volume(s)
}

/// Parse a `--mount type=bind,src=PATH,dst=PATH[,readonly]` spec into a `BindMount`.
pub fn parse_mount(s: &str) -> anyhow::Result<BindMount> {
    BindMount::parse_mount(s)
}

/// Options for the `run` subcommand, bundled to avoid a long parameter list.
// Each bool is an independent CLI flag (--privileged, --tty, --rm, --dind);
// they don't form combinable state and a state machine would just be this struct again.
#[allow(clippy::struct_excessive_bools)]
pub struct RunOpts {
    pub image: String,
    pub tag: String,
    pub command: Vec<String>,
    pub memory_limit_bytes: Option<u64>,
    pub cpu_weight: Option<u64>,
    pub network: String,
    pub privileged: bool,
    pub uid_range_mode: UidRangeMode,
    pub volumes: Vec<String>,
    pub mount_specs: Vec<String>,
    pub name: Option<String>,
    pub tty: bool,
    pub env: Vec<String>,
    pub entrypoint: Option<String>,
    pub user: Option<String>,
    pub auto_remove: bool,
    pub platform: Option<String>,
    pub cgroup_parent: Option<String>,
    pub dind: bool,
    pub dind_debug: bool,
}

/// Nested daemons need full container-creation privileges, so `--dind`
/// implies `--privileged` regardless of whether the flag was passed directly.
const fn effective_privileged(privileged: bool, dind: bool) -> bool {
    privileged || dind
}

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
/// the kernel does not permit overlay-on-overlay. Backing it with the host's
/// (usually tmpfs-backed) temp directory is what makes nested runs work.
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

/// Host-side staging plus the mounts/env a `--dind` run needs.
///
/// The temp directory must outlive the container: the bind mounts point into
/// it, so dropping it early would leave the container reading deleted paths.
struct DindProvisioning {
    /// Keeps the host staging directory alive for the duration of the run.
    _temp_dir: tempfile::TempDir,
    /// Extra bind mounts for the run request.
    mounts: Vec<BindMount>,
    /// Environment the wrapper and any nested `mbx` rely on.
    env: Vec<String>,
}

/// Locate the `miniboxd` binary that will run inside the container.
fn find_miniboxd_binary() -> Result<std::path::PathBuf> {
    if let Ok(path) = which::which("miniboxd") {
        return Ok(path);
    }
    for candidate in ["/usr/local/bin/miniboxd", "/usr/bin/miniboxd"] {
        let path = std::path::PathBuf::from(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }
    anyhow::bail!("miniboxd binary not found on PATH; install miniboxd to use --dind")
}

/// Get the CLI version for protocol version negotiation.
fn get_cli_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Validate the staged miniboxd binary is executable and built for Linux.
///
/// A macOS host binary (Mach-O) or a non-executable file would fail opaquely
/// inside the container, so reject it before the run starts.
fn validate_dind_binary(miniboxd: &std::path::Path) -> Result<()> {
    let metadata = std::fs::metadata(miniboxd)
        .with_context(|| format!("failed to stat miniboxd at {}", miniboxd.display()))?;
    if metadata.permissions().mode() & 0o111 == 0 {
        anyhow::bail!(
            "miniboxd at {} is not executable (mode: {:o})",
            miniboxd.display(),
            metadata.permissions().mode()
        );
    }

    let output = Command::new("file")
        .arg("-b")
        .arg(miniboxd)
        .output()
        .context("failed to run `file` command to check binary type")?;
    let file_info = String::from_utf8_lossy(&output.stdout);

    if !file_info.contains("ELF") {
        anyhow::bail!(
            "miniboxd at {} is not an ELF binary (type: {}) — cannot run in Linux containers; \
            ensure miniboxd is built for the target Linux platform (e.g., x86_64-unknown-linux-musl)",
            miniboxd.display(),
            file_info.trim()
        );
    }

    if file_info.contains("dynamically linked") {
        tracing::warn!(
            "miniboxd is dynamically linked — ensure the container libc (glibc/musl) matches"
        );
    } else if file_info.contains("statically linked") {
        tracing::debug!("miniboxd is statically linked — good for container compatibility");
    }

    Ok(())
}

/// Stage the `DinD` helper files on the host and describe the mounts/env a
/// `--dind` run needs.
///
/// Layout of the staging directory (all mounted into the container):
///
/// * `miniboxd` — nested daemon binary, mounted at [`DIND_MINIBOXD_PATH`]
/// * `mbx-dind-entrypoint` — wrapper script, mounted at [`DIND_WRAPPER_PATH`]
/// * `mbx` — nested CLI, mounted at [`DIND_MBX_PATH`]
/// * `data/`, `run/` — nested daemon state, mounted at the container-side
///   data/run dirs so it never touches the outer container's overlay
fn dind_provision(
    miniboxd_host: &std::path::Path,
    mbx_host: &std::path::Path,
    dind_debug: bool,
) -> Result<DindProvisioning> {
    let temp_dir = tempfile::TempDir::new().context("create DinD staging directory")?;
    let root = temp_dir.path().to_path_buf();

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
            host_path: root,
            container_path: std::path::PathBuf::from(DIND_DIR_PATH),
            read_only: true,
        },
        BindMount {
            host_path: mbx_dest,
            container_path: std::path::PathBuf::from(DIND_MBX_PATH),
            read_only: true,
        },
        BindMount {
            host_path: data_dir,
            container_path: std::path::PathBuf::from(DIND_DATA_DIR_PATH),
            read_only: false,
        },
        BindMount {
            host_path: run_dir,
            container_path: std::path::PathBuf::from(DIND_RUN_DIR_PATH),
            read_only: false,
        },
        BindMount {
            host_path: std::path::PathBuf::from(DIND_CGROUP_FS_PATH),
            container_path: std::path::PathBuf::from(DIND_CGROUP_FS_PATH),
            read_only: false,
        },
    ];

    // NOTE: the outer image cache is deliberately *not* mounted into the
    // nested daemon. Its images dir is `MINIBOX_DATA_DIR/images`, i.e. nested
    // under the `/minibox-data` bind mount, and bind mounts are applied to the
    // rootfs path-by-path before pivot_root — a second mount whose target lies
    // inside an already-mounted bind resolves outside the rootfs and is
    // rejected as a path-traversal escape. The nested daemon therefore pulls
    // into its own (host-backed) data dir.

    // Each run gets its own delegated cgroup slice, so parallel DinD
    // containers never collide on the shared cgroup mount.
    let cgroup_root = format!("{DIND_CGROUP_FS_PATH}/{}", dind_cgroup_slice_name());

    Ok(DindProvisioning {
        _temp_dir: temp_dir,
        mounts,
        env: vec![
            format!("MINIBOX_DATA_DIR={DIND_DATA_DIR_PATH}"),
            format!("MINIBOX_RUN_DIR={DIND_RUN_DIR_PATH}"),
            format!("MINIBOX_SOCKET_PATH={DIND_SOCKET_PATH}"),
            format!("MINIBOX_CGROUP_ROOT={cgroup_root}"),
        ],
    })
}

/// Unique nested cgroup slice name for this CLI process.
fn dind_cgroup_slice_name() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.subsec_nanos());
    format!("minibox-dind-{}-{nanos}", std::process::id())
}

/// Resolve everything a `--dind` run needs before the request is sent.
fn stage_dind(dind_debug: bool) -> Result<DindProvisioning> {
    let miniboxd = find_miniboxd_binary()?;
    validate_dind_binary(&miniboxd).context("DinD binary validation failed")?;
    if !std::path::Path::new(DIND_CGROUP_FS_PATH).is_dir() {
        anyhow::bail!(
            "--dind requires a Linux host with a mounted cgroup v2 filesystem at {DIND_CGROUP_FS_PATH}"
        );
    }
    let mbx = std::env::current_exe().context("resolve current mbx executable for DinD")?;
    dind_provision(&miniboxd, &mbx, dind_debug)
}

/// Entrypoint wrapper script with health monitoring: starts a nested daemon,
/// monitors its health, times out on socket poll, and fails fast on startup errors.
/// Enhanced with pre-flight checks: validates miniboxd executable, /bin/sh presence, and logs progress.
/// When `dind_debug` is true, captures miniboxd stderr for diagnostics.
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

/// Execute the `run` subcommand.
///
/// Connects to the daemon, sends an ephemeral `DaemonRequest::Run`, then
/// streams `ContainerOutput` chunks to stdout/stderr until `ContainerStopped`
/// is received.  Exits with the container's exit code.
pub async fn execute(opts: RunOpts, socket_path: &std::path::Path) -> Result<()> {
    let RunOpts {
        image,
        tag,
        command,
        memory_limit_bytes,
        cpu_weight,
        network,
        privileged,
        uid_range_mode,
        volumes,
        mount_specs,
        name,
        tty,
        env,
        entrypoint,
        user,
        auto_remove,
        platform,
        cgroup_parent,
        dind,
        dind_debug,
    } = opts;
    let privileged = effective_privileged(privileged, dind);

    let network_mode = match network.as_str() {
        "none" => NetworkMode::None,
        "bridge" => NetworkMode::Bridge,
        "host" => NetworkMode::Host,
        "tailnet" => NetworkMode::Tailnet,
        other => {
            anyhow::bail!("unknown network mode: {other} (expected: none, bridge, host, tailnet)")
        }
    };

    // Parse -v shorthand mounts.
    let mut mounts: Vec<BindMount> = Vec::new();
    for v in &volumes {
        mounts.push(parse_volume(v).with_context(|| format!("invalid -v flag {v:?}"))?);
    }
    // Parse --mount long-form mounts.
    for m in &mount_specs {
        mounts.push(parse_mount(m).with_context(|| format!("invalid --mount flag {m:?}"))?);
    }

    let tty = tty && std::io::stdout().is_terminal();

    // DinD: stage the nested daemon binary, wrapper, and nested CLI on the
    // host, then mount them in. The staging directory must outlive the
    // (streaming) run, so it is bound to this scope rather than dropped here.
    let mut env = env;
    let mut entrypoint = entrypoint;
    let _dind_provisioning = if dind {
        let provisioning = stage_dind(dind_debug)?;
        mounts.extend(provisioning.mounts.iter().cloned());
        env.extend(provisioning.env.iter().cloned());
        env.push(format!("MINIBOX_CLIENT_VERSION={}", get_cli_version()));
        entrypoint = Some(DIND_WRAPPER_PATH.to_string());
        Some(provisioning)
    } else {
        None
    };

    let request = DaemonRequest::Run {
        image,
        tag: Some(tag),
        command,
        memory_limit_bytes,
        cpu_weight,
        ephemeral: true,
        network: Some(network_mode),
        mounts,
        privileged,
        shared_uid_range: uid_range_mode == UidRangeMode::Shared,
        env,
        name,
        tty,
        entrypoint,
        user,
        auto_remove,
        priority: None,
        urgency: None,
        execution_context: None,
        platform,
        cgroup_parent,
        dind,
        dind_debug,
        client_version: dind.then(get_cli_version),
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    #[cfg(unix)]
    let mut raw_guard: Option<crate::terminal::RawModeGuard> = None;

    // Stream responses until ContainerStopped or an error.
    while let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ContainerOutput { stream, data } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(&data)
                    .context("failed to decode container output chunk")?;
                match stream {
                    OutputStreamKind::Stdout => {
                        std::io::stdout().write_all(&bytes)?;
                        std::io::stdout().flush()?;
                    }
                    OutputStreamKind::Stderr => {
                        std::io::stderr().write_all(&bytes)?;
                        std::io::stderr().flush()?;
                    }
                }
            }
            DaemonResponse::ContainerStopped { exit_code } => {
                #[cfg(unix)]
                drop(raw_guard);
                std::process::exit(exit_code);
            }
            DaemonResponse::ContainerCreated { id } => {
                // Print the container ID for the caller.  In the ephemeral
                // streaming path this is the first message; ContainerOutput
                // chunks and ContainerStopped follow.  In the non-ephemeral
                // path the daemon closes the channel after this message, so
                // the while-loop exits naturally.
                println!("{id}");
                #[cfg(unix)]
                if tty {
                    raw_guard =
                        Some(crate::terminal::RawModeGuard::enter().context("raw mode enter")?);
                    let (cols, rows) = crate::terminal::terminal_size();
                    let _ = DaemonClient::with_socket(socket_path)
                        .call(DaemonRequest::ResizePty {
                            session_id: minibox_core::domain::SessionId::from(id),
                            cols,
                            rows,
                        })
                        .await;
                }
            }
            DaemonResponse::Error { message } => {
                return Err(RequestError::DaemonError { message }.into());
            }
            other => {
                return Err(RequestError::UnexpectedResponse {
                    response: format!("{other:?}"),
                }
                .into());
            }
        }
    }

    Ok(())
}

#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::redundant_closure_for_method_calls,
    clippy::redundant_clone,
    clippy::single_char_pattern,
    clippy::uninlined_format_args,
    clippy::semicolon_if_nothing_returned,
    clippy::doc_markdown
)]
#[cfg(test)]
mod tests {
    use super::super::test_helpers::setup;
    use super::*;
    use minibox_core::protocol::{DaemonResponse, OutputStreamKind};
    use std::path::PathBuf;

    /// ContainerCreated is the non-streaming legacy path — returns Ok(()) without
    /// calling process::exit, so it's the only execute() path testable in-process.
    #[cfg(unix)]
    #[tokio::test]
    async fn execute_returns_ok_on_container_created() {
        let (_tmp, socket_path) = setup(DaemonResponse::ContainerCreated {
            id: "abc123".to_string(),
        })
        .await;
        let result = execute(
            RunOpts {
                image: "alpine".to_string(),
                tag: "latest".to_string(),
                command: vec!["/bin/sh".to_string()],
                memory_limit_bytes: None,
                cpu_weight: None,
                network: "none".to_string(),
                privileged: false,
                uid_range_mode: UidRangeMode::Exclusive,
                volumes: vec![],
                mount_specs: vec![],
                name: None,
                tty: false,
                env: vec![],
                entrypoint: None,
                user: None,
                auto_remove: false,
                platform: None,
                cgroup_parent: None,
                dind: false,
                dind_debug: false,
            },
            &socket_path,
        )
        .await;
        assert!(
            result.is_ok(),
            "execute should return Ok on ContainerCreated: {result:?}"
        );
    }

    #[test]
    fn test_dind_enables_privileged() {
        assert!(effective_privileged(false, true));
    }

    #[test]
    fn test_dind_false_does_not_force_privileged() {
        assert!(!effective_privileged(false, false));
    }

    #[test]
    fn test_privileged_flag_alone_still_works() {
        assert!(effective_privileged(true, false));
    }

    #[test]
    fn test_dind_entrypoint_container() {
        let script = dind_entrypoint_script_for_container(false);
        assert!(script.starts_with("#!/bin/sh"));
        assert!(script.contains("miniboxd"));
        assert!(script.contains(DIND_SOCKET_PATH));
        assert!(script.contains(DIND_MINIBOXD_PATH));
        assert!(
            script.contains("MINIBOX_CGROUP_ROOT"),
            "wrapper must initialise the nested cgroup slice"
        );
        assert!(
            script.contains(&format!("export MINIBOX_SOCKET_PATH={DIND_SOCKET_PATH}")),
            "nested mbx must resolve the nested socket"
        );
        assert!(
            script.trim_end().ends_with("exec \"$@\""),
            "entrypoint must hand off to the original command: {script}"
        );
    }

    /// Build a [`DindProvisioning`] from dummy host files so the mount and env
    /// layout can be asserted without a real Linux miniboxd binary.
    fn test_dind_provisioning() -> DindProvisioning {
        let src = tempfile::tempdir().expect("temp source dir");
        let miniboxd = src.path().join("miniboxd");
        let mbx = src.path().join("mbx");
        std::fs::write(&miniboxd, b"#!/bin/sh\nexit 0\n").expect("write dummy miniboxd");
        std::fs::write(&mbx, b"#!/bin/sh\nexit 0\n").expect("write dummy mbx");
        dind_provision(&miniboxd, &mbx, false).expect("stage dind provisioning")
    }

    fn mount_for<'a>(provisioning: &'a DindProvisioning, container_path: &str) -> &'a BindMount {
        provisioning
            .mounts
            .iter()
            .find(|mount| mount.container_path.as_path() == std::path::Path::new(container_path))
            .unwrap_or_else(|| panic!("missing mount for {container_path}"))
    }

    fn env_value<'a>(provisioning: &'a DindProvisioning, key: &str) -> Option<&'a str> {
        provisioning
            .env
            .iter()
            .find_map(|entry| entry.strip_prefix(&format!("{key}=")))
    }

    #[test]
    fn test_dind_injects_wrapper() {
        let provisioning = test_dind_provisioning();
        let helpers = mount_for(&provisioning, DIND_DIR_PATH);
        let wrapper = helpers.host_path.join("mbx-dind-entrypoint");
        assert!(
            helpers.read_only,
            "helper binaries are read-only in the container"
        );
        assert!(wrapper.is_file(), "wrapper script must be staged");
        assert_eq!(
            DIND_WRAPPER_PATH,
            format!("{DIND_DIR_PATH}/mbx-dind-entrypoint"),
            "wrapper entrypoint must live in the mounted helper dir"
        );
        let mode = std::fs::metadata(&wrapper)
            .expect("stat wrapper")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "wrapper must be executable");
    }

    #[test]
    fn test_dind_stages_nested_daemon_and_cli() {
        let provisioning = test_dind_provisioning();
        let miniboxd = mount_for(&provisioning, DIND_DIR_PATH)
            .host_path
            .join("miniboxd");
        assert!(
            miniboxd.is_file(),
            "miniboxd must be staged next to the wrapper"
        );
        let mode = std::fs::metadata(&miniboxd)
            .expect("stat miniboxd")
            .permissions()
            .mode();
        assert_eq!(mode & 0o111, 0o111, "miniboxd must be executable");

        let mbx = mount_for(&provisioning, DIND_MBX_PATH);
        assert!(mbx.host_path.is_file(), "nested mbx CLI must be staged");
        assert!(mbx.read_only);
    }

    #[test]
    fn test_dind_mounts_socket() {
        let provisioning = test_dind_provisioning();
        assert_eq!(
            env_value(&provisioning, "MINIBOX_SOCKET_PATH"),
            Some(DIND_SOCKET_PATH)
        );
        let run_dir = mount_for(&provisioning, DIND_RUN_DIR_PATH);
        assert!(
            !run_dir.read_only,
            "nested daemon must be able to create its socket"
        );
        assert!(
            DIND_SOCKET_PATH.starts_with(&format!("{DIND_RUN_DIR_PATH}/")),
            "nested socket must live in the mounted run dir"
        );
    }

    #[test]
    fn test_dind_state_dirs_are_host_backed() {
        let provisioning = test_dind_provisioning();
        for container_path in [DIND_DATA_DIR_PATH, DIND_RUN_DIR_PATH] {
            let mount = mount_for(&provisioning, container_path);
            assert!(
                mount.host_path.is_dir(),
                "{container_path} must be host-backed"
            );
            assert!(
                !mount.read_only,
                "{container_path} must be writable by the nested daemon"
            );
        }
        assert_eq!(
            env_value(&provisioning, "MINIBOX_DATA_DIR"),
            Some(DIND_DATA_DIR_PATH)
        );
        assert_eq!(
            env_value(&provisioning, "MINIBOX_RUN_DIR"),
            Some(DIND_RUN_DIR_PATH)
        );
    }

    #[test]
    fn test_dind_mounts_cgroup_fs() {
        let provisioning = test_dind_provisioning();
        let cgroup = mount_for(&provisioning, DIND_CGROUP_FS_PATH);
        assert!(
            !cgroup.read_only,
            "nested daemon must be able to create and enable its own slice"
        );
        let root = env_value(&provisioning, "MINIBOX_CGROUP_ROOT").expect("cgroup root env");
        assert!(
            root.starts_with(&format!("{DIND_CGROUP_FS_PATH}/minibox-dind-")),
            "nested slice must live under the mounted cgroup fs: {root}"
        );
        assert!(dind_cgroup_slice_name().starts_with("minibox-dind-"));
    }

    #[test]
    fn test_dind_state_dirs_not_nested_under_other_mounts() {
        // Bind mounts are applied one path at a time against the rootfs before
        // pivot_root. A target inside an already-applied bind resolves outside
        // the rootfs and is rejected as a traversal escape, so no provisioned
        // container path may be a descendant of another.
        let provisioning = test_dind_provisioning();
        let container_paths: Vec<&std::path::Path> = provisioning
            .mounts
            .iter()
            .map(|mount| mount.container_path.as_path())
            .collect();
        for (i, outer) in container_paths.iter().enumerate() {
            for (j, inner) in container_paths.iter().enumerate() {
                assert!(
                    i == j || !inner.starts_with(outer),
                    "mount target {} nests inside {}; nested bind mounts are rejected",
                    inner.display(),
                    outer.display()
                );
            }
        }
    }

    #[test]
    fn test_dind_opts() {
        let opts = RunOpts {
            image: "alpine".to_string(),
            tag: "latest".to_string(),
            command: vec!["/bin/sh".to_string()],
            memory_limit_bytes: None,
            cpu_weight: None,
            network: "none".to_string(),
            privileged: false,
            uid_range_mode: UidRangeMode::Exclusive,
            volumes: vec![],
            mount_specs: vec![],
            name: None,
            tty: false,
            env: vec![],
            entrypoint: None,
            user: None,
            auto_remove: false,
            platform: None,
            cgroup_parent: None,
            dind: true,
            dind_debug: false,
        };
        assert!(opts.dind);
    }

    fn network_mode_from_str(s: &str) -> Result<NetworkMode> {
        match s {
            "none" => Ok(NetworkMode::None),
            "bridge" => Ok(NetworkMode::Bridge),
            "host" => Ok(NetworkMode::Host),
            "tailnet" => Ok(NetworkMode::Tailnet),
            other => anyhow::bail!(
                "unknown network mode: {other} (expected: none, bridge, host, tailnet)"
            ),
        }
    }

    #[test]
    fn network_mode_none() {
        assert!(matches!(
            network_mode_from_str("none").unwrap(),
            NetworkMode::None
        ));
    }

    #[test]
    fn network_mode_bridge() {
        assert!(matches!(
            network_mode_from_str("bridge").unwrap(),
            NetworkMode::Bridge
        ));
    }

    #[test]
    fn network_mode_host() {
        assert!(matches!(
            network_mode_from_str("host").unwrap(),
            NetworkMode::Host
        ));
    }

    #[test]
    fn network_mode_tailnet() {
        assert!(matches!(
            network_mode_from_str("tailnet").unwrap(),
            NetworkMode::Tailnet
        ));
    }

    #[test]
    fn network_mode_unknown_errors() {
        let err = network_mode_from_str("docker").unwrap_err();
        assert!(
            err.to_string().contains("unknown network mode"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn parse_volume_valid_rw() {
        let m = parse_volume("/tmp/host:/guest").unwrap();
        assert_eq!(m.host_path, PathBuf::from("/tmp/host"));
        assert_eq!(m.container_path, PathBuf::from("/guest"));
        assert!(!m.read_only);
    }

    #[test]
    fn parse_volume_valid_ro() {
        let m = parse_volume("/tmp/host:/guest:ro").unwrap();
        assert_eq!(m.host_path, PathBuf::from("/tmp/host"));
        assert_eq!(m.container_path, PathBuf::from("/guest"));
        assert!(m.read_only);
    }

    #[test]
    fn parse_volume_missing_colon_errors() {
        let err = parse_volume("/tmp/nocolon").unwrap_err();
        assert!(err.to_string().contains(":"), "expected colon hint: {err}");
    }

    #[test]
    fn parse_volume_relative_dst_errors() {
        let err = parse_volume("/tmp/host:relative/path").unwrap_err();
        assert!(
            err.to_string().contains("absolute") || err.to_string().contains("/"),
            "expected absolute path error: {err}"
        );
    }

    #[test]
    fn parse_mount_valid_bind() {
        let m = parse_mount("type=bind,src=/tmp/host,dst=/guest").unwrap();
        assert_eq!(m.host_path, PathBuf::from("/tmp/host"));
        assert_eq!(m.container_path, PathBuf::from("/guest"));
        assert!(!m.read_only);
    }

    #[test]
    fn parse_mount_readonly() {
        let m = parse_mount("type=bind,src=/tmp/host,dst=/guest,readonly").unwrap();
        assert!(m.read_only);
    }

    #[test]
    fn parse_mount_non_bind_type_errors() {
        let err = parse_mount("type=volume,src=myvolume,dst=/data").unwrap_err();
        assert!(
            err.to_string().contains("bind") || err.to_string().contains("type"),
            "expected bind-only error: {err}"
        );
    }

    #[test]
    fn parse_mount_missing_src_errors() {
        let err = parse_mount("type=bind,dst=/guest").unwrap_err();
        assert!(err.to_string().contains("src"), "expected src error: {err}");
    }

    #[test]
    fn parse_mount_missing_dst_errors() {
        let err = parse_mount("type=bind,src=/tmp/host").unwrap_err();
        assert!(err.to_string().contains("dst"), "expected dst error: {err}");
    }

    /// Verify that a base64-encoded stdout chunk round-trips correctly.
    #[test]
    fn daemon_response_decode_output_chunk() {
        let encoded = base64::engine::general_purpose::STANDARD.encode(b"hello world\n");
        let response = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stdout,
            data: encoded,
        };
        if let DaemonResponse::ContainerOutput { data, .. } = response {
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&data)
                .unwrap();
            assert_eq!(decoded, b"hello world\n");
        } else {
            panic!("expected ContainerOutput");
        }
    }

    // ── Property-based tests ──────────────────────────────────────────────────

    use proptest::prelude::*;

    fn arb_abs_path() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_-]{1,30}".prop_map(|s| format!("/{s}"))
    }

    fn arb_rel_path() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_-]{1,20}"
    }

    proptest! {
        #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

        /// Any `src:abs_dst` spec succeeds with correct fields and read_only=false.
        #[test]
        fn prop_parse_volume_valid_succeeds(
            src in arb_abs_path(),
            dst in arb_abs_path(),
        ) {
            let spec = format!("{src}:{dst}");
            let result = parse_volume(&spec);
            let ok = result.is_ok();
            prop_assert!(ok, "expected Ok for {spec:?}");
            let m = result.unwrap();
            prop_assert_eq!(m.host_path.to_str().unwrap(), src);
            prop_assert_eq!(m.container_path.to_str().unwrap(), dst);
            prop_assert!(!m.read_only);
        }

        /// `:ro` suffix always sets read_only=true.
        #[test]
        fn prop_parse_volume_ro_suffix(src in arb_abs_path(), dst in arb_abs_path()) {
            let m = parse_volume(&format!("{src}:{dst}:ro")).unwrap();
            prop_assert!(m.read_only);
        }

        /// Relative dst always errors.
        #[test]
        fn prop_parse_volume_relative_dst_errors(src in arb_abs_path(), dst in arb_rel_path()) {
            let is_err = parse_volume(&format!("{src}:{dst}")).is_err();
            prop_assert!(is_err);
        }

        /// No colon separator always errors.
        #[test]
        fn prop_parse_volume_no_separator_errors(s in arb_abs_path()) {
            prop_assert!(parse_volume(&s).is_err());
        }

        /// host_path always roundtrips src exactly.
        #[test]
        fn prop_parse_volume_host_path_roundtrips(src in arb_abs_path(), dst in arb_abs_path()) {
            let m = parse_volume(&format!("{src}:{dst}")).unwrap();
            prop_assert_eq!(m.host_path.to_str().unwrap(), src.as_str());
        }

        /// `type=bind,src=ABS,dst=ABS` always succeeds with correct fields.
        #[test]
        fn prop_parse_mount_bind_absolute_succeeds(
            src in arb_abs_path(),
            dst in arb_abs_path(),
        ) {
            let spec = format!("type=bind,src={src},dst={dst}");
            let result = parse_mount(&spec);
            let ok = result.is_ok();
            prop_assert!(ok, "expected Ok for {spec:?}");
            let m = result.unwrap();
            prop_assert_eq!(m.host_path.to_str().unwrap(), src);
            prop_assert_eq!(m.container_path.to_str().unwrap(), dst);
            prop_assert!(!m.read_only);
        }

        /// `,readonly` always sets read_only=true.
        #[test]
        fn prop_parse_mount_readonly_flag(src in arb_abs_path(), dst in arb_abs_path()) {
            let m = parse_mount(&format!("type=bind,src={src},dst={dst},readonly")).unwrap();
            prop_assert!(m.read_only);
        }

        /// `,ro` shorthand also sets read_only=true.
        #[test]
        fn prop_parse_mount_ro_shorthand(src in arb_abs_path(), dst in arb_abs_path()) {
            let m = parse_mount(&format!("type=bind,src={src},dst={dst},ro")).unwrap();
            prop_assert!(m.read_only);
        }

        /// Relative dst always errors.
        #[test]
        fn prop_parse_mount_relative_dst_errors(src in arb_abs_path(), dst in arb_rel_path()) {
            let is_err = parse_mount(&format!("type=bind,src={src},dst={dst}")).is_err();
            prop_assert!(is_err);
        }

        /// Missing src always errors.
        #[test]
        fn prop_parse_mount_missing_src_errors(dst in arb_abs_path()) {
            let is_err = parse_mount(&format!("type=bind,dst={dst}")).is_err();
            prop_assert!(is_err);
        }

        /// Missing dst always errors.
        #[test]
        fn prop_parse_mount_missing_dst_errors(src in arb_abs_path()) {
            let is_err = parse_mount(&format!("type=bind,src={src}")).is_err();
            prop_assert!(is_err);
        }

        /// Non-bind type always errors.
        #[test]
        fn prop_parse_mount_non_bind_type_errors(
            t in "[a-z]{3,10}",
            src in arb_abs_path(),
            dst in arb_abs_path(),
        ) {
            prop_assume!(t != "bind");
            let is_err = parse_mount(&format!("type={t},src={src},dst={dst}")).is_err();
            prop_assert!(is_err);
        }
    }

    /// Verify that a base64-encoded stderr chunk round-trips and retains the
    /// correct stream kind discriminant.
    #[test]
    fn daemon_response_decode_stderr_chunk() {
        let encoded =
            base64::engine::general_purpose::STANDARD.encode(b"error: something went wrong\n");
        let response = DaemonResponse::ContainerOutput {
            stream: OutputStreamKind::Stderr,
            data: encoded,
        };
        if let DaemonResponse::ContainerOutput { stream, data } = response {
            assert_eq!(stream, OutputStreamKind::Stderr);
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(&data)
                .unwrap();
            assert_eq!(decoded, b"error: something went wrong\n");
        } else {
            panic!("expected ContainerOutput");
        }
    }

    #[test]
    fn test_dind_entrypoint_has_health_monitoring() {
        let script = dind_entrypoint_script_for_container(false);
        assert!(
            script.contains("DAEMON_PID"),
            "entrypoint must capture daemon PID for health monitoring"
        );
        assert!(
            script.contains("kill -0"),
            "entrypoint must check if daemon started with kill -0"
        );
        assert!(
            script.contains("TIMEOUT"),
            "entrypoint must have a timeout for socket poll"
        );
        assert!(
            script.contains("ERROR"),
            "entrypoint must provide error messages"
        );
        assert!(
            script.contains("exit 126"),
            "entrypoint must exit with 126 on daemon startup failure"
        );
        assert!(
            script.contains("exit 127"),
            "entrypoint must exit with 127 on socket timeout"
        );
    }
}
