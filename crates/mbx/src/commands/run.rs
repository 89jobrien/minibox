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
#[cfg(test)]
use std::path::PathBuf;

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
#[allow(dead_code)]
const fn effective_privileged(privileged: bool, dind: bool) -> bool {
    privileged || dind
}

/// Path to the host's daemon Unix socket, bind-mounted 1:1 into the
/// container so a nested `mbx` CLI inside can reach the host daemon.
#[allow(dead_code)]
const DAEMON_SOCKET_PATH: &str = "/run/minibox/miniboxd.sock";

/// Path the miniboxd binary is installed to inside the container.
#[allow(dead_code)]
const DIND_MINIBOXD_PATH: &str = "/usr/local/lib/mbx-dind/miniboxd";

/// Path the dind entrypoint wrapper script is installed to inside the
/// container.
#[allow(dead_code)]
const DIND_WRAPPER_PATH: &str = "/usr/local/lib/mbx-dind/mbx-dind-entrypoint";

/// Path to the shared image cache inside the container.
#[allow(dead_code)]
const DIND_IMAGE_CACHE_CONTAINER_PATH: &str = "/var/lib/minibox/images";

/// Entrypoint override to use when `--dind` is set.
#[allow(dead_code)]
fn dind_entrypoint_override(dind: bool) -> Option<&'static str> {
    dind.then_some(DIND_WRAPPER_PATH)
}

/// Entrypoint wrapper script with health monitoring: starts a nested daemon,
/// monitors its health, times out on socket poll, and fails fast on startup errors.
/// Enhanced with pre-flight checks: validates miniboxd executable, /bin/sh presence, and logs progress.
/// When dind_debug is true, captures miniboxd stderr for diagnostics.
#[allow(dead_code)]
fn dind_entrypoint_script_for_container(dind_debug: bool) -> String {
    let miniboxd_path = DIND_MINIBOXD_PATH;
    let sock = DAEMON_SOCKET_PATH;
    let debug_stderr_redirect = if dind_debug {
        " 2>/tmp/miniboxd-debug.log"
    } else {
        " 2>/dev/null"
    };
    format!(
        "#!/bin/sh\n\
         set -e\n\
         \n\
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
    let privileged = privileged || dind;

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
                dind: false,
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
        assert!(script.contains(DAEMON_SOCKET_PATH));
        assert!(script.contains(DIND_MINIBOXD_PATH));
        assert!(
            script.trim_end().ends_with("exec \"$@\""),
            "entrypoint must hand off to the original command: {script}"
        );
    }

    #[test]
    fn test_dind_injects_wrapper() {
        assert_eq!(dind_entrypoint_override(true), Some(DIND_WRAPPER_PATH));
    }

    #[test]
    fn test_dind_false_does_not_override_entrypoint() {
        assert_eq!(dind_entrypoint_override(false), None);
    }

    #[test]
    fn test_dind_image_cache_path_is_set() {
        // Must match where the nested miniboxd resolves its own images dir
        // (resolve_data_dir().join("images")), or the shared cache mounts to
        // an empty location the nested daemon never looks at.
        assert_eq!(
            DIND_IMAGE_CACHE_CONTAINER_PATH, "/var/lib/minibox/images",
            "dind image cache should mount at /var/lib/minibox/images"
        );
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
