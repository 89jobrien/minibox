//! Shared test helpers for miniboxd integration and e2e tests.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Binary resolution
// ---------------------------------------------------------------------------

/// Find a minibox binary by name.
///
/// Search order:
/// 1. `MINIBOX_TEST_BIN_DIR` env var (set by justfile)
/// 2. `target/release/{name}`
/// 3. `target/debug/{name}`
pub fn find_binary(name: &str) -> PathBuf {
    if let Ok(dir) = std::env::var("MINIBOX_TEST_BIN_DIR") {
        let p = PathBuf::from(&dir).join(name);
        if p.exists() {
            return p;
        }
    }

    // Try CARGO_TARGET_DIR if set (e.g. ~/.minibox/cache/target/)
    let cargo_target_dir = std::env::var("CARGO_TARGET_DIR").ok().map(PathBuf::from);

    if let Some(ref target_dir) = cargo_target_dir {
        for profile in ["release", "debug"] {
            let p = target_dir.join(profile).join(name);
            if p.exists() {
                return p;
            }
        }
    }

    // Try relative to workspace root (CARGO_MANIFEST_DIR points to miniboxd crate)
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
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
        "Could not find binary '{}'. Run `cargo build --release` first, \
         or set MINIBOX_TEST_BIN_DIR.",
        name
    );
}

// ---------------------------------------------------------------------------
// DaemonFixture
// ---------------------------------------------------------------------------

/// RAII fixture that starts a real miniboxd and provides CLI access.
pub struct DaemonFixture {
    pub child: Option<Child>,
    pub socket_path: PathBuf,
    pub data_dir: TempDir,
    pub run_dir: TempDir,
    pub cgroup_root: PathBuf,
    pub cli_bin: PathBuf,
}

impl DaemonFixture {
    /// Start a daemon with isolated temp dirs.
    ///
    /// Panics if the daemon fails to start within 10 seconds.
    pub fn start() -> Self {
        let data_dir = TempDir::with_prefix("minibox-test-data-").expect("create temp data dir");
        let run_dir = TempDir::with_prefix("minibox-test-run-").expect("create temp run dir");

        let socket_path = run_dir.path().join("miniboxd.sock");

        // Create a top-level cgroup slice for this test.  Nesting under the
        // runner's own cgroup violates cgroup v2's "no internal process" rule
        // when the runner is a leaf cgroup (has processes in cgroup.procs),
        // causing create_dir_all to fail with EBUSY.  A top-level slice under
        // /sys/fs/cgroup/ has no processes and allows free sub-cgroup creation.
        let test_name = format!(
            "minibox-test-e2e-{}",
            &uuid::Uuid::new_v4().to_string()[..8]
        );
        let cgroup_root = PathBuf::from("/sys/fs/cgroup").join(&test_name);

        let daemon_bin = find_binary("miniboxd");
        let cli_bin = find_binary("minibox");

        // Create cgroup root and enable the controllers containers need.
        std::fs::create_dir_all(&cgroup_root).ok();
        // Enable memory, cpu, and pids subtree controllers so the daemon can
        // apply resource limits to container cgroups created inside this slice.
        let _ = std::fs::write(
            cgroup_root.join("cgroup.subtree_control"),
            "+memory +cpu +pids",
        );

        let child = Command::new(&daemon_bin)
            .env("MINIBOX_DATA_DIR", data_dir.path())
            .env("MINIBOX_RUN_DIR", run_dir.path())
            .env("MINIBOX_SOCKET_PATH", &socket_path)
            .env("MINIBOX_CGROUP_ROOT", &cgroup_root)
            .env("RUST_LOG", "miniboxd=debug")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to start miniboxd at {:?}: {e}", daemon_bin));

        let fixture = Self {
            child: Some(child),
            socket_path: socket_path.clone(),
            data_dir,
            run_dir,
            cgroup_root,
            cli_bin,
        };

        // Wait for socket to appear
        let start = Instant::now();
        let timeout = Duration::from_secs(10);
        while !socket_path.exists() {
            if start.elapsed() > timeout {
                // Kill and capture stderr for debugging
                let mut fixture = fixture;
                let stderr = fixture.kill_and_capture_stderr();
                panic!(
                    "miniboxd did not create socket within 10s.\nSocket: {:?}\nStderr:\n{}",
                    socket_path, stderr
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }

        fixture
    }

    /// Return the daemon's PID.
    pub fn daemon_pid(&self) -> u32 {
        self.child.as_ref().expect("daemon child missing").id()
    }

    /// Create a Command for the minibox CLI pre-configured with our socket.
    pub fn cli(&self, args: &[&str]) -> Command {
        let mut cmd = Command::new(&self.cli_bin);
        cmd.env("MINIBOX_SOCKET_PATH", &self.socket_path);
        cmd.args(args);
        cmd
    }

    /// Pull an image, panicking with diagnostics on failure.
    ///
    /// Use this in tests that require an image but aren't testing the pull itself.
    pub fn pull_required(&self, image: &str) {
        let (success, stdout, stderr) = self.run_cli(&["pull", image]);
        assert!(
            success,
            "prerequisite image pull failed for '{image}'.\nstdout: {stdout}\nstderr: {stderr}"
        );
    }

    /// Spawn `minibox run <run_args>` in the background and return the
    /// container ID immediately.
    ///
    /// The daemon emits `ContainerCreated { id }` as the first line of stdout
    /// in the streaming path.  This helper reads that line, then drains
    /// remaining stdout in a background thread so the CLI does not stall on a
    /// full pipe buffer.
    ///
    /// `run_args` is everything after `minibox run`, e.g.
    /// `&["alpine", "--memory", "134217728", "--", "/bin/sleep", "30"]`.
    ///
    /// The returned `Child` is still alive (CLI blocks until the container
    /// exits or is stopped).  The caller must stop the container and wait.
    pub fn spawn_run_background(&self, run_args: &[&str]) -> (Child, String) {
        use std::io::{BufRead, BufReader};

        let mut cli_args = vec!["run"];
        cli_args.extend_from_slice(run_args);

        let mut child = self
            .cli(&cli_args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|e| panic!("failed to spawn minibox run {run_args:?}: {e}"));

        let stdout = child.stdout.take().expect("no stdout pipe");
        let mut reader = BufReader::new(stdout);
        let mut first_line = String::new();
        reader
            .read_line(&mut first_line)
            .expect("failed to read container ID from minibox run stdout");
        let container_id = first_line.trim().to_string();
        assert!(
            !container_id.is_empty(),
            "minibox run did not emit a container ID on stdout line 1"
        );

        // Drain remaining stdout in a background thread so the CLI does not
        // block writing ContainerOutput to a full pipe and stall the test.
        std::thread::spawn(move || {
            use std::io::Read;
            let mut buf = [0u8; 4096];
            loop {
                match reader.get_mut().read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
        });

        (child, container_id)
    }

    /// Convenience wrapper around [`spawn_run_background`] for the simple case
    /// of running a command inside a named image with no extra CLI flags.
    pub fn spawn_container_background(&self, image: &str, cmd: &[&str]) -> (Child, String) {
        let mut run_args = vec![image, "--"];
        run_args.extend_from_slice(cmd);
        self.spawn_run_background(&run_args)
    }

    /// Wait for a container to appear as Running in `ps`.
    ///
    /// Returns `true` if seen within timeout, `false` otherwise.
    pub fn wait_for_running(&self, container_id: &str, timeout: Duration) -> bool {
        poll_until(timeout, Duration::from_millis(100), || {
            let (ok, stdout, _) = self.run_cli(&["ps"]);
            ok && stdout.contains(container_id) && stdout.contains("Running")
        })
    }

    /// Run a CLI command and return (exit_status, stdout, stderr).
    pub fn run_cli(&self, args: &[&str]) -> (bool, String, String) {
        let output = self
            .cli(args)
            .output()
            .unwrap_or_else(|e| panic!("failed to run minibox {:?}: {e}", args));

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.success(), stdout, stderr)
    }

    /// Run a CLI command and return (exit_code, stdout, stderr).
    pub fn run_cli_with_exit_code(&self, args: &[&str]) -> (i32, String, String) {
        let output = self
            .cli(args)
            .output()
            .unwrap_or_else(|e| panic!("failed to run minibox {:?}: {e}", args));
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        (output.status.code().unwrap_or(-1), stdout, stderr)
    }

    /// Kill daemon and capture stderr for debugging.
    /// Only call when the daemon is expected to have failed.
    pub fn kill_and_capture_stderr(&mut self) -> String {
        let mut child = match self.child.take() {
            Some(child) => child,
            None => return "(daemon already reaped)".to_string(),
        };
        let _ = child.kill();
        let output = child.wait_with_output();
        match output {
            Ok(o) => String::from_utf8_lossy(&o.stderr).to_string(),
            Err(e) => format!("(could not capture stderr: {e})"),
        }
    }

    /// Send SIGTERM to the daemon.
    pub fn sigterm(&self) {
        let child = self.child.as_ref().expect("daemon child missing");
        // SAFETY: Sending SIGTERM to our known child process PID. The PID is valid
        // because we spawned it and haven't yet waited on it.
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
        }
    }
}

impl Drop for DaemonFixture {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        // 1. Send SIGTERM
        // SAFETY: Sending signal to our known child process PID.
        unsafe {
            libc::kill(child.id() as i32, libc::SIGTERM);
        }

        // 2. Wait up to 5s for clean exit
        let start = Instant::now();
        loop {
            match child.try_wait() {
                Ok(Some(_)) => break,
                Ok(None) => {
                    if start.elapsed() > Duration::from_secs(5) {
                        // 3. Escalate to SIGKILL
                        let _ = child.kill();
                        let _ = child.wait();
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(100));
                }
                Err(_) => break,
            }
        }

        // 4. Cleanup cgroup tree (cgroupfs only supports rmdir, not rm -rf)
        if self.cgroup_root.exists() {
            // Remove leaf cgroups first (children), then root.
            // cgroupfs requires directories to be empty (no child cgroups).
            if let Ok(entries) = std::fs::read_dir(&self.cgroup_root) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        // Recurse one level for nested cgroups (e.g., supervisor/)
                        if let Ok(sub_entries) = std::fs::read_dir(&path) {
                            for sub in sub_entries.flatten() {
                                if sub.path().is_dir() {
                                    let _ = std::fs::remove_dir(&sub.path());
                                }
                            }
                        }
                        let _ = std::fs::remove_dir(&path);
                    }
                }
            }
            let _ = std::fs::remove_dir(&self.cgroup_root);
        }

        // 5. TempDir handles data_dir and run_dir
    }
}

// ---------------------------------------------------------------------------
// Polling helpers
// ---------------------------------------------------------------------------

/// Poll `condition` every `interval` until it returns true or `timeout` elapses.
///
/// Returns `true` if the condition was satisfied, `false` on timeout.
pub fn poll_until(
    timeout: Duration,
    interval: Duration,
    mut condition: impl FnMut() -> bool,
) -> bool {
    let start = Instant::now();
    loop {
        if condition() {
            return true;
        }
        if start.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(interval);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Try to extract a container ID from CLI output.
///
/// Looks for a 16-char hex string (the truncated UUID format used by minibox).
pub fn extract_container_id(output: &str) -> Option<String> {
    // Look for a hex-like ID in the output
    for word in output.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if cleaned.len() == 16 && cleaned.chars().all(|c| c.is_ascii_hexdigit()) {
            return Some(cleaned.to_string());
        }
    }
    // Fallback: look for any alphanumeric token of length 16
    for word in output.split_whitespace() {
        let cleaned = word.trim_matches(|c: char| !c.is_ascii_alphanumeric());
        if cleaned.len() == 16 && cleaned.chars().all(|c| c.is_ascii_alphanumeric()) {
            return Some(cleaned.to_string());
        }
    }
    None
}

// ---------------------------------------------------------------------------
// SandboxClient — agent-oriented wrapper around DaemonFixture
// ---------------------------------------------------------------------------

/// Structured result from a sandbox execution.
pub struct ExecResult {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub duration: Duration,
}

/// Agent-oriented test client wrapping DaemonFixture.
pub struct SandboxClient {
    pub fixture: DaemonFixture,
    pulled_images: HashSet<String>,
}

impl SandboxClient {
    pub fn start() -> Self {
        Self {
            fixture: DaemonFixture::start(),
            pulled_images: HashSet::new(),
        }
    }

    fn parse_image_tag(image: &str) -> (&str, &str) {
        match image.split_once(':') {
            Some((img, tag)) => (img, tag),
            None => (image, "latest"),
        }
    }

    pub fn ensure_image(&mut self, image: &str) {
        if self.pulled_images.contains(image) {
            return;
        }
        let (img, tag) = Self::parse_image_tag(image);
        let (ok, _stdout, stderr) = self.fixture.run_cli(&["pull", img, "--tag", tag]);
        assert!(ok, "failed to pull {image}: {stderr}");
        self.pulled_images.insert(image.to_string());
    }

    pub fn execute(&mut self, image: &str, cmd: &[&str]) -> ExecResult {
        self.ensure_image(image);
        let (img, tag) = Self::parse_image_tag(image);
        let start = Instant::now();
        let mut args = vec!["run", img, "--tag", tag, "--"];
        args.extend_from_slice(cmd);
        let (exit_code, stdout, stderr) = self.fixture.run_cli_with_exit_code(&args);
        ExecResult {
            stdout,
            stderr,
            exit_code,
            duration: start.elapsed(),
        }
    }

    pub fn execute_with_limits(
        &mut self,
        image: &str,
        cmd: &[&str],
        memory_bytes: u64,
        cpu_weight: u64,
    ) -> ExecResult {
        self.ensure_image(image);
        let (img, tag) = Self::parse_image_tag(image);
        let mem_str = memory_bytes.to_string();
        let cpu_str = cpu_weight.to_string();
        let start = Instant::now();
        let mut args = vec![
            "run",
            img,
            "--tag",
            tag,
            "--memory",
            &mem_str,
            "--cpu-weight",
            &cpu_str,
            "--",
        ];
        args.extend_from_slice(cmd);
        let (exit_code, stdout, stderr) = self.fixture.run_cli_with_exit_code(&args);
        ExecResult {
            stdout,
            stderr,
            exit_code,
            duration: start.elapsed(),
        }
    }

    pub fn spawn_container(&mut self, image: &str, cmd: &[&str]) -> Child {
        self.ensure_image(image);
        let (img, tag) = Self::parse_image_tag(image);
        let mut args = vec!["run", img, "--tag", tag, "--"];
        args.extend_from_slice(cmd);
        self.fixture
            .cli(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("failed to spawn container")
    }
}
