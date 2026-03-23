//! minibox-bench — benchmark harness for the minibox container runtime.
//!
//! # Suites
//!
//! * **codec** — protocol encode/decode (nanosecond-scale, 36 cases).  Runs
//!   `encode_request`, `decode_request`, `encode_response`, `decode_response`
//!   for every [`DaemonRequest`] / [`DaemonResponse`] variant in both small
//!   and large payload configurations.  Requires no daemon.
//!
//! * **adapter** — trait-object dispatch overhead (nanosecond-scale, 10
//!   cases).  Compares direct method calls on concrete mock types against the
//!   same calls through `Arc<dyn Trait>` pointers.  Requires no daemon.
//!
//! * **pull** / **run** / **ps** / **stop** / **rm** / **exec** / **e2e** —
//!   end-to-end suites that invoke the `minibox` CLI binary. `exec` is kept as
//!   a `run`-with-output variant to measure stdout capture overhead. These
//!   suites require a running `miniboxd` daemon.
//!
//! # Output
//!
//! Results are saved to `bench/results/` (or `--out-dir`):
//!
//! * `bench.jsonl` — append-only history managed by `cargo xtask bench`.
//! * `latest.json` — canonical snapshot of the most recent run.
//! * A timestamped JSON + plain-text table pair produced by each invocation.
//!
//! The path of the timestamped JSON is printed to stdout so callers such as
//! `xtask` can capture it without scanning the output directory.

use minibox_lib::adapters::mocks::{MockFilesystem, MockLimiter, MockRegistry, MockRuntime};
use minibox_lib::domain::{
    ContainerHooks, ContainerRuntime, ContainerSpawnConfig, FilesystemProvider, ImageRegistry,
    ResourceConfig, ResourceLimiter,
};
use minibox_lib::protocol::{
    ContainerInfo, DAEMON_SOCKET_PATH, DaemonRequest, DaemonResponse, decode_request,
    decode_response, encode_request, encode_response,
};
use serde::Serialize;
use std::hint::black_box;
use std::path::PathBuf;
use std::sync::Arc;

/// Top-level benchmark report serialised to JSON.
///
/// Contains run metadata, one [`SuiteResult`] per executed suite, and a
/// de-duplicated list of errors collected during the run.
#[derive(Serialize, Default)]
struct BenchReport {
    metadata: Metadata,
    suites: Vec<SuiteResult>,
    errors: Vec<ErrorCount>,
}

impl BenchReport {
    /// Construct an empty report with default metadata.  Used in tests.
    #[allow(dead_code)]
    fn empty() -> Self {
        Self::default()
    }
}

/// A single error message together with the number of times it occurred.
///
/// Repeated identical errors are folded into one entry by [`dedup_errors`] to
/// keep the JSON report compact.
#[derive(Serialize, Clone)]
struct ErrorCount {
    /// Human-readable error description.
    message: String,
    /// Number of times this exact message was recorded.
    count: usize,
}

/// Fold a flat list of error strings into de-duplicated [`ErrorCount`] entries.
///
/// Preserves insertion order of first occurrence; later duplicates increment
/// the count of the existing entry.
fn dedup_errors(raw: Vec<String>) -> Vec<ErrorCount> {
    let mut seen: Vec<(String, usize)> = Vec::new();
    for msg in raw {
        if let Some(entry) = seen.iter_mut().find(|(m, _)| m == &msg) {
            entry.1 += 1;
        } else {
            seen.push((msg, 1));
        }
    }
    seen.into_iter()
        .map(|(message, count)| ErrorCount { message, count })
        .collect()
}

/// Run-level metadata embedded in every [`BenchReport`].
#[derive(Serialize, Default)]
struct Metadata {
    /// RFC 3339 timestamp of when the benchmark run started.
    timestamp: String,
    /// Hostname of the machine that produced this report.
    hostname: String,
    /// Full `git rev-parse HEAD` SHA of the minibox tree being benchmarked.
    git_sha: String,
    /// Output of `minibox --version` for the binary under test.
    minibox_version: String,
}

/// Results for a single named benchmark suite (e.g. `"codec"`, `"adapter"`).
#[derive(Serialize, Default)]
struct SuiteResult {
    /// Suite identifier (e.g. `"codec"`, `"adapter"`, `"pull"`).
    name: String,
    /// Individual test cases within this suite.
    tests: Vec<TestResult>,
}

/// Timing data and summary statistics for a single benchmark case.
///
/// Exactly one of `durations_micros` and `durations_nanos` will be populated
/// depending on the resolution needed by the suite.  The `unit` field
/// discriminates which: `"nanos"` for nanosecond-resolution micro-benchmarks
/// and `"micros"` (omitted, i.e. the default) for end-to-end CLI timings.
#[derive(Serialize, Default)]
struct TestResult {
    /// Unique name for this test case within its suite.
    name: String,
    /// Number of iterations actually executed.
    iterations: usize,
    /// Per-iteration wall-clock durations in microseconds.  Populated for
    /// end-to-end CLI suites (`pull`, `run`, `exec`, `e2e`).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    durations_micros: Vec<u64>,
    /// Per-iteration wall-clock durations in nanoseconds.  Populated for
    /// nanosecond-resolution micro-benchmark suites (`codec`, `adapter`).
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    durations_nanos: Vec<u64>,
    /// Aggregate statistics computed from whichever duration vector is
    /// populated.  `None` if no iterations completed successfully.
    stats: Option<Stats>,
    /// Duration unit used for display: `"nanos"` for nanosecond suites;
    /// empty string (omitted from JSON) for microsecond suites.
    #[serde(skip_serializing_if = "String::is_empty", default)]
    unit: String,
}

/// Serialise `report` to pretty-printed JSON and write it to `path`.
fn write_json(report: &BenchReport, path: &str) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(report).unwrap();
    std::fs::write(path, json)
}

/// Format `report` as a tab-separated plain-text table and write it to `path`.
///
/// Nanosecond-resolution results are displayed as integer nanoseconds; all
/// other results are displayed in milliseconds with three decimal places.
fn write_table(report: &BenchReport, path: &str) -> std::io::Result<()> {
    let mut out = String::new();
    out.push_str("minibox benchmark results\n\n");
    out.push_str(&format!("suites: {}\n\n", report.suites.len()));
    out.push_str("suite\ttest\titers\tunit\tmin\tavg\tp95\n");
    for suite in &report.suites {
        for test in &suite.tests {
            let is_nanos = test.unit == "nanos";
            let (unit_label, min, avg, p95) = match test.stats.as_ref() {
                Some(s) if is_nanos => (
                    "ns",
                    format!("{}", s.min),
                    format!("{}", s.avg),
                    format!("{}", s.p95),
                ),
                Some(s) => (
                    "ms",
                    format!("{:.3}", s.min as f64 / 1000.0),
                    format!("{:.3}", s.avg as f64 / 1000.0),
                    format!("{:.3}", s.p95 as f64 / 1000.0),
                ),
                None => (
                    "n/a",
                    "n/a".to_string(),
                    "n/a".to_string(),
                    "n/a".to_string(),
                ),
            };
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                suite.name, test.name, test.iterations, unit_label, min, avg, p95
            ));
        }
    }
    std::fs::write(path, out)
}

/// Output and timing captured from a single external command invocation.
#[derive(Debug)]
struct CmdResult {
    /// Whether the command exited with a zero status code.
    success: bool,
    /// Contents of the command's stdout, decoded as lossy UTF-8.
    stdout: String,
    /// Contents of the command's stderr, decoded as lossy UTF-8.
    stderr: String,
    /// Wall-clock duration of the invocation in microseconds.
    duration_micros: u64,
}

/// Run an external command and return its output and wall-clock duration.
fn run_cmd(path: &str, args: &[&str]) -> std::io::Result<CmdResult> {
    let start = std::time::Instant::now();
    let output = std::process::Command::new(path).args(args).output()?;
    let duration_micros = start.elapsed().as_micros() as u64;
    Ok(CmdResult {
        success: output.status.success(),
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
        duration_micros,
    })
}

/// Parsed command-line configuration controlling which suites run and how.
#[derive(Debug, Clone, PartialEq)]
struct BenchConfig {
    /// Number of iterations per test case (default: 20; micro-benchmark suites
    /// use at least 100 regardless).
    iters: usize,
    /// Whether to run "cold" suites (`pull`, `e2e`).
    cold: bool,
    /// Whether to run "warm" suites (`run`, `exec`).
    warm: bool,
    /// Skip actual execution and produce an empty report (for smoke-testing).
    dry_run: bool,
    /// Explicit list of suite names from `--suite`; if non-empty, overrides
    /// the `cold`/`warm` defaults.
    suites: Vec<String>,
    /// Directory where timestamped JSON and text results are written.
    out_dir: String,
    /// Whether to attach `perf record` (Linux) for each suite.
    profile: bool,
}

impl BenchConfig {
    /// Parse `BenchConfig` from `std::env::args()`.
    ///
    /// Returns an error string describing the first unrecognised argument or
    /// missing value.
    fn from_args(args: Vec<String>) -> Result<Self, String> {
        let mut iters = 20usize;
        let mut cold = true;
        let mut warm = true;
        let mut dry_run = false;
        let mut suites = Vec::new();
        let mut out_dir = "bench/results".to_string();
        let mut profile = false;

        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--iters" => {
                    i += 1;
                    let val = args
                        .get(i)
                        .ok_or_else(|| "--iters requires a value".to_string())?;
                    iters = val.parse().map_err(|_| "invalid --iters".to_string())?;
                }
                "--cold" => cold = true,
                "--no-cold" => cold = false,
                "--warm" => warm = true,
                "--no-warm" => warm = false,
                "--dry-run" => dry_run = true,
                "--profile" => profile = true,
                "--suite" => {
                    i += 1;
                    let val = args
                        .get(i)
                        .ok_or_else(|| "--suite requires a value".to_string())?;
                    suites.push(val.to_string());
                }
                "--out-dir" => {
                    i += 1;
                    let val = args
                        .get(i)
                        .ok_or_else(|| "--out-dir requires a value".to_string())?;
                    out_dir = val.to_string();
                }
                "--help" | "-h" => {
                    print_help();
                    std::process::exit(0);
                }
                other => return Err(format!("unknown argument: {other}")),
            }
            i += 1;
        }

        Ok(Self {
            iters,
            cold,
            warm,
            dry_run,
            suites,
            out_dir,
            profile,
        })
    }

    /// Return the default configuration.  Used in tests that do not want to
    /// go through argument parsing.
    #[allow(dead_code)]
    fn default() -> Self {
        Self {
            iters: 20,
            cold: true,
            warm: true,
            dry_run: false,
            suites: Vec::new(),
            out_dir: "bench/results".to_string(),
            profile: false,
        }
    }
}

/// Aggregate statistics computed from a sample vector of durations.
///
/// All values are in the same unit as the input samples (nanoseconds or
/// microseconds depending on the suite).
#[derive(Debug, PartialEq, Serialize)]
struct Stats {
    /// Minimum observed duration.
    min: u64,
    /// Arithmetic mean of all samples.
    avg: u64,
    /// 95th-percentile duration (sorted index `ceil(0.95 * (n-1))`).
    p95: u64,
}

impl Stats {
    /// Compute min, average, and p95 from `samples`.
    ///
    /// Sorts a copy of the slice internally; the original is not modified.
    /// Returns zeroes for all fields if `samples` is empty.
    fn from_samples(samples: &[u64]) -> Self {
        let mut sorted = samples.to_vec();
        sorted.sort_unstable();
        let min = *sorted.first().unwrap_or(&0);
        let sum: u64 = sorted.iter().sum();
        let avg = if sorted.is_empty() {
            0
        } else {
            sum / sorted.len() as u64
        };
        let p95_idx = if sorted.is_empty() {
            0
        } else {
            ((sorted.len() - 1) as f64 * 0.95).ceil() as usize
        };
        let p95 = *sorted.get(p95_idx).unwrap_or(&0);
        Self { min, avg, p95 }
    }
}

/// Return `Some(Stats)` for a non-empty slice, or `None` if the slice is empty.
fn stats_for(samples: &[u64]) -> Option<Stats> {
    if samples.is_empty() {
        None
    } else {
        Some(Stats::from_samples(samples))
    }
}

// ── Profiler trait and implementations ────────────────────────────────────────

/// Path to a pprof-format profile file produced by a [`Profiler`].
#[derive(Debug, Clone)]
pub struct ProfilePath {
    /// Path to the `.pprof` file (may or may not exist if conversion failed).
    pub pprof_path: PathBuf,
}

/// Errors that can occur while starting or stopping a profiler.
#[derive(Debug)]
pub enum ProfileError {
    /// The current platform does not support the requested profiler.
    PlatformNotSupported,
    /// The `perf` binary could not be found in `$PATH`.
    PerfBinaryNotFound,
    /// `perf` was found but returned a non-zero exit status or could not be
    /// spawned.
    PerfFailed(String),
    /// A filesystem I/O error occurred while managing profiler output files.
    IoError(String),
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProfileError::PlatformNotSupported => {
                write!(f, "profiling not supported on this platform")
            }
            ProfileError::PerfBinaryNotFound => write!(f, "perf binary not found"),
            ProfileError::PerfFailed(msg) => write!(f, "perf failed: {}", msg),
            ProfileError::IoError(msg) => write!(f, "io error: {}", msg),
        }
    }
}

/// Trait for platform-specific CPU profilers.
///
/// Implementations bracket a benchmark suite with `start` and `stop` calls.
/// Non-fatal issues (e.g. `perf` not installed) are logged as warnings and
/// the benchmarks continue; only hard errors are propagated.
pub trait Profiler: Send {
    /// Begin profiling the named suite.
    fn start(&mut self, suite: &str) -> Result<(), ProfileError>;
    /// Stop profiling the named suite and return the path to the profile file.
    fn stop(&mut self, suite: &str) -> Result<ProfilePath, ProfileError>;
}

/// Linux `perf record` profiler.
///
/// Spawns `perf record -p <bench_pid>` for each suite and kills it on
/// [`stop`](Profiler::stop).  If `pprof` is also available the raw
/// `perf.data` is converted to pprof format.
///
/// Instantiated only on Linux; see [`create_profiler`].
pub struct LinuxPerfProfiler {
    /// Directory where `<suite>.perf.data` and `<suite>.pprof` files are written.
    results_dir: PathBuf,
    /// RFC 3339 timestamp passed in at construction.  Reserved for future use
    /// in perf output filenames; currently not read after construction.
    #[allow(dead_code)] // reserved for future perf profiling output filename disambiguation
    timestamp: String,
    /// Map from suite name to the live `perf record` child process.
    perf_pids: std::collections::HashMap<String, std::process::Child>,
}

impl LinuxPerfProfiler {
    /// Create a new profiler that writes profile data under `results_dir`.
    ///
    /// `timestamp` is accepted for future use in disambiguating output file
    /// names but is not currently read after construction.
    pub fn new(results_dir: PathBuf, timestamp: String) -> Self {
        Self {
            results_dir,
            timestamp,
            perf_pids: std::collections::HashMap::new(),
        }
    }

    /// Return `true` if `perf` is present in `$PATH`.
    fn check_perf_available() -> bool {
        std::process::Command::new("which")
            .arg("perf")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    }
}

impl Profiler for LinuxPerfProfiler {
    /// Attach `perf record` to the current process for the named suite.
    ///
    /// If `perf` is not in `$PATH` the call succeeds with a warning printed to
    /// stderr and no child process is spawned.
    fn start(&mut self, suite: &str) -> Result<(), ProfileError> {
        if !Self::check_perf_available() {
            eprintln!(
                "warn: perf binary not found, skipping profile for suite '{}'",
                suite
            );
            return Ok(());
        }

        let perf_data_path = self.results_dir.join(format!("{}.perf.data", suite));
        let bench_pid = std::process::id();

        let perf_child = std::process::Command::new("perf")
            .args(&[
                "record",
                "-p",
                &bench_pid.to_string(),
                "-o",
                perf_data_path.to_str().unwrap_or(""),
            ])
            .spawn()
            .map_err(|e| ProfileError::PerfFailed(format!("failed to spawn perf: {}", e)))?;

        self.perf_pids.insert(suite.to_string(), perf_child);
        Ok(())
    }

    /// Kill the `perf record` child for `suite`, wait for it to exit, and
    /// optionally convert the raw `perf.data` to pprof format.
    ///
    /// If no child was recorded for `suite` (e.g. `perf` was absent) the call
    /// succeeds and returns a path that may not exist on disk.
    fn stop(&mut self, suite: &str) -> Result<ProfilePath, ProfileError> {
        if let Some(mut child) = self.perf_pids.remove(suite) {
            child
                .kill()
                .map_err(|e| ProfileError::PerfFailed(format!("failed to kill perf: {}", e)))?;
            child
                .wait()
                .map_err(|e| ProfileError::PerfFailed(format!("perf wait failed: {}", e)))?;

            let perf_data = self.results_dir.join(format!("{}.perf.data", suite));
            let pprof_path = self.results_dir.join(format!("{}.pprof", suite));

            // Convert perf.data to pprof (optional - if pprof tool not available, just return path)
            if std::process::Command::new("which")
                .arg("pprof")
                .output()
                .map(|out| out.status.success())
                .unwrap_or(false)
            {
                let _convert = std::process::Command::new("pprof")
                    .args(&["-proto", perf_data.to_str().unwrap_or("")])
                    .output()
                    .ok();
            }

            Ok(ProfilePath { pprof_path })
        } else {
            Ok(ProfilePath {
                pprof_path: self.results_dir.join(format!("{}.pprof", suite)),
            })
        }
    }
}

/// No-op profiler used on macOS (and any platform where Linux `perf` is
/// unavailable).  All calls succeed immediately with a warning.
pub struct MacOSProfiler;

impl Profiler for MacOSProfiler {
    /// No-op on macOS; emits a warning and returns `Ok(())`.
    fn start(&mut self, suite: &str) -> Result<(), ProfileError> {
        eprintln!(
            "warn: profiling not supported on macOS; skipping profile for suite '{}'",
            suite
        );
        Ok(())
    }

    /// No-op on macOS; returns a placeholder path with the `.pprof` extension.
    fn stop(&mut self, suite: &str) -> Result<ProfilePath, ProfileError> {
        Ok(ProfilePath {
            pprof_path: PathBuf::from(format!("{}.pprof", suite)),
        })
    }
}

/// Construct the appropriate [`Profiler`] for the current platform.
///
/// On Linux returns a [`LinuxPerfProfiler`]; on all other platforms returns a
/// [`MacOSProfiler`] (no-op).
pub fn create_profiler(results_dir: PathBuf, timestamp: String) -> Box<dyn Profiler> {
    if cfg!(target_os = "linux") {
        Box::new(LinuxPerfProfiler::new(results_dir, timestamp))
    } else {
        Box::new(MacOSProfiler)
    }
}

/// Return `true` if the named suite should run given `cfg`.
///
/// If `cfg.suites` is non-empty only the explicitly named suites run.
/// Otherwise `"pull"` and `"e2e"` run when `cfg.cold` is set; `"run"` and
/// the command-oriented warm suites (`"ps"`, `"stop"`, `"rm"`, `"exec"`) run
/// when `cfg.warm` is set. `"codec"` and `"adapter"` only run when explicitly
/// listed in `--suite`.
fn suite_enabled(cfg: &BenchConfig, name: &str) -> bool {
    if !cfg.suites.is_empty() {
        return cfg.suites.iter().any(|suite| suite == name);
    }

    match name {
        "pull" | "e2e" => cfg.cold,
        "run" | "ps" | "stop" | "rm" | "exec" => cfg.warm,
        // codec, adapter, colima only run when explicitly requested via --suite
        "codec" | "adapter" | "colima" => false,
        _ => true,
    }
}

/// Return the ordered list of suite names that will run for `cfg`.
fn planned_suites(cfg: &BenchConfig) -> Vec<String> {
    let mut suites = Vec::new();
    for name in [
        "pull", "run", "ps", "stop", "rm", "exec", "e2e", "codec", "adapter", "colima",
    ] {
        if suite_enabled(cfg, name) {
            suites.push(name.to_string());
        }
    }
    suites
}

/// Run a command, capture stdout/stderr, trim whitespace, and return the first
/// non-empty result.  Returns `None` if the command fails or produces no
/// output.
fn read_cmd_trim(path: &str, args: &[&str]) -> Option<String> {
    let output = std::process::Command::new(path).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if !stdout.is_empty() {
        return Some(stdout);
    }
    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    if stderr.is_empty() {
        None
    } else {
        Some(stderr)
    }
}

/// Gather run metadata: current timestamp, hostname, git SHA, and the version
/// string reported by `minibox_bin --version`.
fn build_metadata(minibox_bin: &str) -> Metadata {
    Metadata {
        timestamp: chrono::Utc::now().to_rfc3339(),
        hostname: read_cmd_trim("hostname", &[]).unwrap_or_else(|| "unknown".to_string()),
        git_sha: read_cmd_trim("git", &["rev-parse", "HEAD"])
            .unwrap_or_else(|| "unknown".to_string()),
        minibox_version: read_cmd_trim(minibox_bin, &["--version"])
            .unwrap_or_else(|| "unknown".to_string()),
    }
}

#[cfg(unix)]
fn daemon_socket_path() -> String {
    std::env::var("MINIBOX_SOCKET_PATH").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/tmp/minibox/miniboxd.sock".to_string()
        } else {
            DAEMON_SOCKET_PATH.to_string()
        }
    })
}

#[cfg(not(unix))]
fn daemon_socket_path() -> String {
    std::env::var("MINIBOX_SOCKET_PATH").unwrap_or_else(|_| DAEMON_SOCKET_PATH.to_string())
}

#[cfg(unix)]
fn send_daemon_request_sync(request: &DaemonRequest) -> Result<DaemonResponse, String> {
    use std::io::{BufRead, BufReader, Write};
    use std::os::unix::net::UnixStream;

    let path = daemon_socket_path();
    let mut stream =
        UnixStream::connect(&path).map_err(|e| format!("connecting to daemon at {path}: {e}"))?;
    let payload = encode_request(request).map_err(|e| format!("encoding request: {e}"))?;
    stream
        .write_all(&payload)
        .map_err(|e| format!("writing request to daemon: {e}"))?;
    stream
        .flush()
        .map_err(|e| format!("flushing request to daemon: {e}"))?;

    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    reader
        .read_until(b'\n', &mut line)
        .map_err(|e| format!("reading response from daemon: {e}"))?;
    if line.is_empty() {
        return Err("daemon closed connection without sending a response".to_string());
    }
    decode_response(&line).map_err(|e| format!("decoding daemon response: {e}"))
}

#[cfg(not(unix))]
fn send_daemon_request_sync(_request: &DaemonRequest) -> Result<DaemonResponse, String> {
    Err("daemon fixture setup only supports Unix sockets".to_string())
}

fn list_containers_sync() -> Result<Vec<ContainerInfo>, String> {
    match send_daemon_request_sync(&DaemonRequest::List)? {
        DaemonResponse::ContainerList { containers } => Ok(containers),
        DaemonResponse::Error { message } => Err(format!("daemon list request failed: {message}")),
        other => Err(format!("unexpected list response: {other:?}")),
    }
}

fn wait_for_container_presence(id: &str, should_exist: bool) -> Result<(), String> {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        let present = list_containers_sync()?
            .iter()
            .any(|container| container.id == id);
        if present == should_exist {
            return Ok(());
        }
        if std::time::Instant::now() >= deadline {
            let expectation = if should_exist {
                "appear in"
            } else {
                "disappear from"
            };
            return Err(format!(
                "container {id} did not {expectation} daemon state in time"
            ));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

fn create_background_container() -> Result<String, String> {
    let request = DaemonRequest::Run {
        image: "alpine".to_string(),
        tag: Some("latest".to_string()),
        command: vec![
            "/bin/sh".to_string(),
            "-c".to_string(),
            "sleep 60".to_string(),
        ],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: false,
        network: None,
    };

    match send_daemon_request_sync(&request)? {
        DaemonResponse::ContainerCreated { id } => {
            wait_for_container_presence(&id, true)?;
            Ok(id)
        }
        DaemonResponse::Error { message } => {
            Err(format!("daemon background run failed: {message}"))
        }
        other => Err(format!("unexpected background run response: {other:?}")),
    }
}

fn stop_container_direct(id: &str) -> Result<(), String> {
    match send_daemon_request_sync(&DaemonRequest::Stop { id: id.to_string() })? {
        DaemonResponse::Success { .. } => Ok(()),
        DaemonResponse::Error { message } => Err(format!("daemon stop failed for {id}: {message}")),
        other => Err(format!("unexpected stop response for {id}: {other:?}")),
    }
}

fn remove_container_direct(id: &str) -> Result<(), String> {
    match send_daemon_request_sync(&DaemonRequest::Remove { id: id.to_string() })? {
        DaemonResponse::Success { .. } => {
            wait_for_container_presence(id, false)?;
            Ok(())
        }
        DaemonResponse::Error { message } => Err(format!("daemon rm failed for {id}: {message}")),
        other => Err(format!("unexpected remove response for {id}: {other:?}")),
    }
}

fn cleanup_background_container(id: &str, errors: &mut Vec<String>) {
    let containers = match list_containers_sync() {
        Ok(containers) => containers,
        Err(err) => {
            errors.push(format!(
                "fixture cleanup: failed to list containers for {id}: {err}"
            ));
            return;
        }
    };

    let Some(container) = containers.into_iter().find(|container| container.id == id) else {
        return;
    };

    if container.state == "Running"
        && let Err(err) = stop_container_direct(id)
    {
        errors.push(format!("fixture cleanup: failed to stop {id}: {err}"));
    }

    if let Err(err) = remove_container_direct(id) {
        errors.push(format!("fixture cleanup: failed to remove {id}: {err}"));
    }
}

fn run_cmd_record_owned(
    path: &str,
    args: &[String],
    errors: &mut Vec<String>,
) -> Option<CmdResult> {
    let arg_refs: Vec<&str> = args.iter().map(String::as_str).collect();
    run_cmd_record(path, &arg_refs, errors)
}

/// Run an external command, appending a descriptive message to `errors` on
/// failure.  Returns `Some(CmdResult)` on success or `None` if the command
/// failed or could not be spawned.
fn run_cmd_record(path: &str, args: &[&str], errors: &mut Vec<String>) -> Option<CmdResult> {
    match run_cmd(path, args) {
        Ok(result) => {
            if !result.success {
                let stderr = result.stderr.trim();
                let stdout = result.stdout.trim();
                let mut message = format!("command failed: {path} {args:?}");
                if !stderr.is_empty() {
                    message.push_str(&format!("\nstderr: {stderr}"));
                }
                if !stdout.is_empty() {
                    message.push_str(&format!("\nstdout: {stdout}"));
                }
                errors.push(message);
                return None;
            }
            Some(result)
        }
        Err(err) => {
            errors.push(format!("command error: {path} {args:?}: {err}"));
            None
        }
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let cfg = match BenchConfig::from_args(args) {
        Ok(cfg) => cfg,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(2);
        }
    };

    let report = match run_benchmark(&cfg) {
        Ok(r) => r,
        Err(err) => {
            eprintln!("error: {err}");
            std::process::exit(1);
        }
    };

    let timestamp = chrono::Utc::now().to_rfc3339();
    let json_path = format!("{}/{}.json", cfg.out_dir, timestamp);
    let table_path = format!("{}/{}.txt", cfg.out_dir, timestamp);
    if let Err(e) = std::fs::create_dir_all(&cfg.out_dir) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
    if let Err(e) = write_json(&report, &json_path) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
    // Print JSON path to stdout so callers (e.g. xtask) can capture it without scanning the dir.
    println!("{json_path}");
    if let Err(e) = write_table(&report, &table_path) {
        eprintln!("error: {e}");
        std::process::exit(1);
    }
}

/// Resolve the results directory and timestamp, then delegate to [`run_suites`].
fn run_benchmark(cfg: &BenchConfig) -> Result<BenchReport, String> {
    let results_dir = PathBuf::from(&cfg.out_dir);
    let timestamp = chrono::Utc::now().to_rfc3339();
    run_suites(cfg, cfg.dry_run, &results_dir, &timestamp)
}

/// Sidecar metadata written alongside profile data when `--profile` is active.
#[derive(Serialize)]
struct ProfileMetadata {
    /// RFC 3339 timestamp of the profiling run.
    timestamp: String,
    /// Full git SHA of the tree that was profiled.
    git_sha: String,
    /// Names of suites for which profiling was attempted.
    suites_profiled: Vec<String>,
    /// Platform string (`"linux"`, `"macos"`, or `"unknown"`).
    platform: String,
    /// Whether `perf` was found in `$PATH` on this run.
    perf_available: bool,
}

/// Write `metadata.json` alongside profile output in `results_dir`.
fn save_profile_metadata(
    results_dir: &PathBuf,
    timestamp: &str,
    suites: &[String],
) -> Result<(), String> {
    let git_sha =
        read_cmd_trim("git", &["rev-parse", "HEAD"]).unwrap_or_else(|| "unknown".to_string());

    let perf_available = if cfg!(target_os = "linux") {
        std::process::Command::new("which")
            .arg("perf")
            .output()
            .map(|out| out.status.success())
            .unwrap_or(false)
    } else {
        false
    };

    let platform = if cfg!(target_os = "linux") {
        "linux".to_string()
    } else if cfg!(target_os = "macos") {
        "macos".to_string()
    } else {
        "unknown".to_string()
    };

    let metadata = ProfileMetadata {
        timestamp: timestamp.to_string(),
        git_sha,
        suites_profiled: suites.to_vec(),
        platform,
        perf_available,
    };

    let metadata_json = serde_json::to_string_pretty(&metadata)
        .map_err(|e| format!("failed to serialize profile metadata: {}", e))?;

    let metadata_path = results_dir.join("metadata.json");
    std::fs::write(&metadata_path, metadata_json)
        .map_err(|e| format!("failed to write metadata.json: {}", e))?;

    Ok(())
}

/// Execute all enabled suites and return the combined [`BenchReport`].
///
/// If `dry_run` is `true` the suites are enumerated in the report but no
/// commands are executed and no timings are recorded.
fn run_suites(
    cfg: &BenchConfig,
    dry_run: bool,
    results_dir: &PathBuf,
    timestamp: &str,
) -> Result<BenchReport, String> {
    let minibox_bin = std::env::var("MINIBOX_BIN").unwrap_or_else(|_| "minibox".to_string());
    let metadata = build_metadata(&minibox_bin);
    let selected_suites = planned_suites(cfg);
    let mut suites = Vec::new();
    let mut errors = Vec::new();

    // Create timestamp subdirectory for profiles if profiling is enabled
    let profile_dir = if cfg.profile {
        let dir = results_dir.join(timestamp);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("warn: failed to create profile directory: {}", e);
        }
        Some(dir)
    } else {
        None
    };

    let mut profiler: Option<Box<dyn Profiler>> = if let Some(ref dir) = profile_dir {
        Some(create_profiler(dir.clone(), timestamp.to_string()))
    } else {
        None
    };

    if selected_suites.is_empty() {
        errors.push("no suites selected (use --suite or enable --cold/--warm)".to_string());
    }

    if dry_run {
        for suite in selected_suites {
            suites.push(SuiteResult {
                name: suite,
                tests: Vec::new(),
            });
        }
        return Ok(BenchReport {
            metadata,
            suites,
            errors: dedup_errors(errors),
        });
    }

    if suite_enabled(cfg, "pull") {
        let suite_name = "pull";
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.start(suite_name) {
                eprintln!(
                    "warn: failed to start profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }

        let mut pull_suite = SuiteResult {
            name: "pull".to_string(),
            tests: Vec::new(),
        };
        let mut pull_durations = Vec::with_capacity(5);
        for _ in 0..5 {
            if let Some(r) = run_cmd_record(&minibox_bin, &["pull", "alpine"], &mut errors) {
                pull_durations.push(r.duration_micros);
            }
        }
        pull_suite.tests.push(TestResult {
            name: "pull_alpine".to_string(),
            iterations: pull_durations.len(),
            durations_micros: pull_durations.clone(),
            stats: stats_for(&pull_durations),
            ..Default::default()
        });

        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.stop(suite_name) {
                eprintln!(
                    "warn: failed to stop profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        suites.push(pull_suite);
    }

    if suite_enabled(cfg, "run") {
        let suite_name = "run";
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.start(suite_name) {
                eprintln!(
                    "warn: failed to start profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }

        let mut run_suite = SuiteResult {
            name: "run".to_string(),
            tests: Vec::new(),
        };
        let mut durations = Vec::with_capacity(cfg.iters);
        for _ in 0..cfg.iters {
            if let Some(run) = run_cmd_record(
                &minibox_bin,
                &["run", "alpine", "--", "/bin/true"],
                &mut errors,
            ) {
                durations.push(run.duration_micros);
            }
        }
        run_suite.tests.push(TestResult {
            name: "run_true".to_string(),
            iterations: durations.len(),
            durations_micros: durations.clone(),
            stats: stats_for(&durations),
            ..Default::default()
        });

        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.stop(suite_name) {
                eprintln!(
                    "warn: failed to stop profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        suites.push(run_suite);
    }

    if suite_enabled(cfg, "ps") {
        let suite_name = "ps";
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.start(suite_name) {
                eprintln!(
                    "warn: failed to start profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }

        let mut ps_suite = SuiteResult {
            name: "ps".to_string(),
            tests: Vec::new(),
        };
        let mut durations = Vec::with_capacity(cfg.iters);
        match create_background_container() {
            Ok(id) => {
                for _ in 0..cfg.iters {
                    if let Some(result) = run_cmd_record(&minibox_bin, &["ps"], &mut errors) {
                        if result.stdout.contains(&id) {
                            durations.push(result.duration_micros);
                        } else {
                            errors.push(format!(
                                "command failed: {minibox_bin} [\"ps\"]\nstdout did not include fixture container id {id}"
                            ));
                        }
                    }
                }
                cleanup_background_container(&id, &mut errors);
            }
            Err(err) => errors.push(format!("ps fixture setup failed: {err}")),
        }
        ps_suite.tests.push(TestResult {
            name: "ps_one_running".to_string(),
            iterations: durations.len(),
            durations_micros: durations.clone(),
            stats: stats_for(&durations),
            ..Default::default()
        });

        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.stop(suite_name) {
                eprintln!(
                    "warn: failed to stop profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        suites.push(ps_suite);
    }

    if suite_enabled(cfg, "stop") {
        let suite_name = "stop";
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.start(suite_name) {
                eprintln!(
                    "warn: failed to start profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }

        let mut stop_suite = SuiteResult {
            name: "stop".to_string(),
            tests: Vec::new(),
        };
        let mut durations = Vec::with_capacity(cfg.iters);
        for _ in 0..cfg.iters {
            match create_background_container() {
                Ok(id) => {
                    let args = vec!["stop".to_string(), id.clone()];
                    if let Some(result) = run_cmd_record_owned(&minibox_bin, &args, &mut errors) {
                        if result.stdout.contains(&id) {
                            durations.push(result.duration_micros);
                        } else {
                            errors.push(format!(
                                "command failed: {minibox_bin} {:?}\nstdout did not include stopped container id {id}",
                                args
                            ));
                        }
                    }
                    cleanup_background_container(&id, &mut errors);
                }
                Err(err) => errors.push(format!("stop fixture setup failed: {err}")),
            }
        }
        stop_suite.tests.push(TestResult {
            name: "stop_sleep".to_string(),
            iterations: durations.len(),
            durations_micros: durations.clone(),
            stats: stats_for(&durations),
            ..Default::default()
        });

        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.stop(suite_name) {
                eprintln!(
                    "warn: failed to stop profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        suites.push(stop_suite);
    }

    if suite_enabled(cfg, "rm") {
        let suite_name = "rm";
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.start(suite_name) {
                eprintln!(
                    "warn: failed to start profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }

        let mut rm_suite = SuiteResult {
            name: "rm".to_string(),
            tests: Vec::new(),
        };
        let mut durations = Vec::with_capacity(cfg.iters);
        for _ in 0..cfg.iters {
            match create_background_container() {
                Ok(id) => {
                    if let Err(err) = stop_container_direct(&id) {
                        errors.push(format!("rm fixture stop failed for {id}: {err}"));
                        cleanup_background_container(&id, &mut errors);
                        continue;
                    }
                    let args = vec!["rm".to_string(), id.clone()];
                    if let Some(result) = run_cmd_record_owned(&minibox_bin, &args, &mut errors) {
                        if result.stdout.contains(&id) {
                            durations.push(result.duration_micros);
                        } else {
                            errors.push(format!(
                                "command failed: {minibox_bin} {:?}\nstdout did not include removed container id {id}",
                                args
                            ));
                        }
                    }
                    cleanup_background_container(&id, &mut errors);
                }
                Err(err) => errors.push(format!("rm fixture setup failed: {err}")),
            }
        }
        rm_suite.tests.push(TestResult {
            name: "rm_stopped".to_string(),
            iterations: durations.len(),
            durations_micros: durations.clone(),
            stats: stats_for(&durations),
            ..Default::default()
        });

        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.stop(suite_name) {
                eprintln!(
                    "warn: failed to stop profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        suites.push(rm_suite);
    }

    if suite_enabled(cfg, "exec") {
        let suite_name = "exec";
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.start(suite_name) {
                eprintln!(
                    "warn: failed to start profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }

        let mut exec_suite = SuiteResult {
            name: "exec".to_string(),
            tests: Vec::new(),
        };
        let mut durations = Vec::with_capacity(cfg.iters);
        for _ in 0..cfg.iters {
            if let Some(exec) = run_cmd_record(
                &minibox_bin,
                &["run", "alpine", "--", "/bin/echo", "ok"],
                &mut errors,
            ) {
                durations.push(exec.duration_micros);
            }
        }
        exec_suite.tests.push(TestResult {
            name: "exec_echo".to_string(),
            iterations: durations.len(),
            durations_micros: durations.clone(),
            stats: stats_for(&durations),
            ..Default::default()
        });

        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.stop(suite_name) {
                eprintln!(
                    "warn: failed to stop profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        suites.push(exec_suite);
    }

    if suite_enabled(cfg, "e2e") {
        let suite_name = "e2e";
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.start(suite_name) {
                eprintln!(
                    "warn: failed to start profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }

        let mut e2e_suite = SuiteResult {
            name: "e2e".to_string(),
            tests: Vec::new(),
        };
        let mut e2e_pull_dur = Vec::with_capacity(5);
        for _ in 0..5 {
            if let Some(r) = run_cmd_record(&minibox_bin, &["pull", "alpine"], &mut errors) {
                e2e_pull_dur.push(r.duration_micros);
            }
        }
        e2e_suite.tests.push(TestResult {
            name: "pull_alpine".to_string(),
            iterations: e2e_pull_dur.len(),
            durations_micros: e2e_pull_dur.clone(),
            stats: stats_for(&e2e_pull_dur),
            ..Default::default()
        });
        let mut e2e_run_dur = Vec::with_capacity(5);
        for _ in 0..5 {
            if let Some(r) = run_cmd_record(
                &minibox_bin,
                &["run", "alpine", "--", "/bin/true"],
                &mut errors,
            ) {
                e2e_run_dur.push(r.duration_micros);
            }
        }
        e2e_suite.tests.push(TestResult {
            name: "run_true".to_string(),
            iterations: e2e_run_dur.len(),
            durations_micros: e2e_run_dur.clone(),
            stats: stats_for(&e2e_run_dur),
            ..Default::default()
        });

        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.stop(suite_name) {
                eprintln!(
                    "warn: failed to stop profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        suites.push(e2e_suite);
    }

    if suite_enabled(cfg, "codec") {
        let suite_name = "codec";
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.start(suite_name) {
                eprintln!(
                    "warn: failed to start profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        let codec_suite = bench_codec_suite(cfg);
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.stop(suite_name) {
                eprintln!(
                    "warn: failed to stop profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        suites.push(codec_suite);
    }

    if suite_enabled(cfg, "adapter") {
        let suite_name = "adapter";
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.start(suite_name) {
                eprintln!(
                    "warn: failed to start profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        let adapter_suite = bench_adapter_suite(cfg);
        if let Some(ref mut prof) = profiler {
            if let Err(e) = prof.stop(suite_name) {
                eprintln!(
                    "warn: failed to stop profiler for suite {}: {}",
                    suite_name, e
                );
            }
        }
        suites.push(adapter_suite);
    }

    if suite_enabled(cfg, "colima") {
        suites.push(bench_colima_suite(cfg, &minibox_bin, &mut errors));
    }

    let report = BenchReport {
        metadata,
        suites,
        errors: dedup_errors(errors),
    };

    // Save profiling metadata if enabled
    if let Some(ref dir) = profile_dir {
        save_profile_metadata(dir, timestamp, &planned_suites(cfg))?;
    }

    Ok(report)
}

// ── Colima suite (macOS + Colima running) ────────────────────────────────────

/// Check if Colima is installed and running (macOS only).
fn colima_available() -> bool {
    cfg!(target_os = "macos")
        && std::process::Command::new("colima")
            .arg("status")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
}

fn bench_colima_suite(
    cfg: &BenchConfig,
    minibox_bin: &str,
    errors: &mut Vec<String>,
) -> SuiteResult {
    let iters = cfg.iters.max(3);
    let mut tests = Vec::new();

    if !colima_available() {
        eprintln!("warn: colima suite skipped — Colima not running or not on macOS");
        return SuiteResult {
            name: "colima".to_string(),
            tests: vec![],
        };
    }

    // 1. Limactl round-trip baseline
    {
        let mut durations = Vec::with_capacity(iters);
        for _ in 0..iters {
            let start = std::time::Instant::now();
            let _ = std::process::Command::new("limactl")
                .args(["shell", "colima", "echo", "ok"])
                .output();
            durations.push(start.elapsed().as_micros() as u64);
        }
        tests.push(TestResult {
            name: "limactl-roundtrip".to_string(),
            iterations: durations.len(),
            durations_micros: durations.clone(),
            stats: stats_for(&durations),
            ..Default::default()
        });
    }

    // 2. Spawn-to-first-byte (requires running miniboxd)
    {
        let mut durations = Vec::with_capacity(iters);
        for _ in 0..iters {
            if let Some(r) = run_cmd_record(
                minibox_bin,
                &["run", "alpine", "--", "/bin/echo", "bench-marker"],
                errors,
            ) {
                if r.success && r.stdout.contains("bench-marker") {
                    durations.push(r.duration_micros);
                }
            }
        }
        tests.push(TestResult {
            name: "spawn-to-first-byte".to_string(),
            iterations: durations.len(),
            durations_micros: durations.clone(),
            stats: stats_for(&durations),
            ..Default::default()
        });
    }

    // 3. Stop latency (start a long-running container, then stop it)
    {
        let mut durations = Vec::with_capacity(iters);
        for _ in 0..iters {
            // Start a background container (non-ephemeral returns container ID)
            if let Ok(run_result) = run_cmd(minibox_bin, &["run", "alpine", "--", "sleep", "300"]) {
                if run_result.success {
                    let container_id = run_result.stdout.trim().to_string();
                    if !container_id.is_empty() {
                        let start = std::time::Instant::now();
                        let _ = run_cmd(minibox_bin, &["stop", &container_id]);
                        durations.push(start.elapsed().as_micros() as u64);
                        // Clean up
                        let _ = run_cmd(minibox_bin, &["rm", &container_id]);
                    }
                }
            }
        }
        tests.push(TestResult {
            name: "stop-latency".to_string(),
            iterations: durations.len(),
            durations_micros: durations.clone(),
            stats: stats_for(&durations),
            ..Default::default()
        });
    }

    SuiteResult {
        name: "colima".to_string(),
        tests,
    }
}

// ── Microbenchmark suites (no daemon required) ───────────────────────────────

/// Run `f` exactly `iters` times, recording the wall-clock duration of each
/// call in nanoseconds.  Returns one sample per iteration.
fn measure_nanos<F: FnMut()>(iters: usize, mut f: F) -> Vec<u64> {
    (0..iters)
        .map(|_| {
            let start = std::time::Instant::now();
            f();
            start.elapsed().as_nanos() as u64
        })
        .collect()
}

/// Build a [`TestResult`] for a nanosecond-resolution micro-benchmark.
///
/// Populates `durations_nanos` and sets `unit = "nanos"`.
fn nano_test(name: &str, iters: usize, f: impl FnMut()) -> TestResult {
    let durations = measure_nanos(iters, f);
    let stats = stats_for(&durations);
    TestResult {
        name: name.to_string(),
        iterations: durations.len(),
        durations_nanos: durations,
        stats,
        unit: "nanos".to_string(),
        ..Default::default()
    }
}

/// Run the `codec` micro-benchmark suite.
///
/// Measures [`encode_request`], [`decode_request`], [`encode_response`], and
/// [`decode_response`] for every [`DaemonRequest`] / [`DaemonResponse`] variant
/// in both small and large payload configurations.  36 cases total.  No daemon
/// required.
fn bench_codec_suite(cfg: &BenchConfig) -> SuiteResult {
    let iters = cfg.iters.max(100);

    // ── requests ─────────────────────────────────────────────────────────────
    let small_run = DaemonRequest::Run {
        image: "alpine".to_string(),
        tag: None,
        command: vec!["/bin/sh".to_string()],
        memory_limit_bytes: None,
        cpu_weight: None,
        ephemeral: false,
        network: None,
    };
    let large_run = DaemonRequest::Run {
        image: "library/some-really-long-image-name-for-benchmarking".to_string(),
        tag: Some("2026.03.16-benchmarks".to_string()),
        command: (0..24)
            .map(|i| format!("arg-{}-{}", i, "x".repeat(16)))
            .collect(),
        memory_limit_bytes: Some(512 * 1024 * 1024),
        cpu_weight: Some(7500),
        ephemeral: false,
        network: None,
    };
    let small_pull = DaemonRequest::Pull {
        image: "alpine".to_string(),
        tag: None,
    };
    let large_pull = DaemonRequest::Pull {
        image: "library/some-really-long-image-name-for-benchmarking".to_string(),
        tag: Some("2026.03.16-benchmarks".to_string()),
    };
    let small_stop = DaemonRequest::Stop {
        id: "deadbeefdeadbeef".to_string(),
    };
    let large_stop = DaemonRequest::Stop {
        id: "deadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    };
    let small_remove = DaemonRequest::Remove {
        id: "deadbeefdeadbeef".to_string(),
    };
    let large_remove = DaemonRequest::Remove {
        id: "deadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    };
    let list_req = DaemonRequest::List;

    // ── responses ────────────────────────────────────────────────────────────
    fn make_container_info(i: usize) -> ContainerInfo {
        ContainerInfo {
            id: format!("{:016x}", i),
            image: format!("library/image-{}", i),
            command: format!("echo hello {}", i),
            state: if i.is_multiple_of(2) {
                "running"
            } else {
                "stopped"
            }
            .to_string(),
            created_at: format!("2026-03-16T12:{:02}:00Z", i % 60),
            pid: Some(1000 + i as u32),
        }
    }
    let small_created = DaemonResponse::ContainerCreated {
        id: "deadbeefdeadbeef".to_string(),
    };
    let large_created = DaemonResponse::ContainerCreated {
        id: "deadbeefdeadbeefdeadbeefdeadbeef".to_string(),
    };
    let small_success = DaemonResponse::Success {
        message: "ok".to_string(),
    };
    let large_success = DaemonResponse::Success {
        message: "operation completed successfully with additional context".to_string(),
    };
    let small_error = DaemonResponse::Error {
        message: "error".to_string(),
    };
    let large_error = DaemonResponse::Error {
        message: "error: failed to perform operation due to invalid state".to_string(),
    };
    let small_list = DaemonResponse::ContainerList {
        containers: vec![make_container_info(0)],
    };
    let large_list = DaemonResponse::ContainerList {
        containers: (0..100).map(make_container_info).collect(),
    };

    // Pre-encode for decode benchmarks.
    let enc_small_run = encode_request(&small_run).unwrap();
    let enc_large_run = encode_request(&large_run).unwrap();
    let enc_small_pull = encode_request(&small_pull).unwrap();
    let enc_large_pull = encode_request(&large_pull).unwrap();
    let enc_small_stop = encode_request(&small_stop).unwrap();
    let enc_large_stop = encode_request(&large_stop).unwrap();
    let enc_small_remove = encode_request(&small_remove).unwrap();
    let enc_large_remove = encode_request(&large_remove).unwrap();
    let enc_list = encode_request(&list_req).unwrap();
    let enc_small_created = encode_response(&small_created).unwrap();
    let enc_large_created = encode_response(&large_created).unwrap();
    let enc_small_success = encode_response(&small_success).unwrap();
    let enc_large_success = encode_response(&large_success).unwrap();
    let enc_small_error = encode_response(&small_error).unwrap();
    let enc_large_error = encode_response(&large_error).unwrap();
    let enc_small_list = encode_response(&small_list).unwrap();
    let enc_large_list = encode_response(&large_list).unwrap();

    let mut tests = vec![
        nano_test("encode_run_small", iters, || {
            black_box(encode_request(black_box(&small_run)).unwrap());
        }),
        nano_test("decode_run_small", iters, || {
            black_box(decode_request(black_box(&enc_small_run)).unwrap());
        }),
        nano_test("encode_run_large", iters, || {
            black_box(encode_request(black_box(&large_run)).unwrap());
        }),
        nano_test("decode_run_large", iters, || {
            black_box(decode_request(black_box(&enc_large_run)).unwrap());
        }),
        nano_test("encode_pull_small", iters, || {
            black_box(encode_request(black_box(&small_pull)).unwrap());
        }),
        nano_test("decode_pull_small", iters, || {
            black_box(decode_request(black_box(&enc_small_pull)).unwrap());
        }),
        nano_test("encode_pull_large", iters, || {
            black_box(encode_request(black_box(&large_pull)).unwrap());
        }),
        nano_test("decode_pull_large", iters, || {
            black_box(decode_request(black_box(&enc_large_pull)).unwrap());
        }),
        nano_test("encode_stop_small", iters, || {
            black_box(encode_request(black_box(&small_stop)).unwrap());
        }),
        nano_test("decode_stop_small", iters, || {
            black_box(decode_request(black_box(&enc_small_stop)).unwrap());
        }),
        nano_test("encode_stop_large", iters, || {
            black_box(encode_request(black_box(&large_stop)).unwrap());
        }),
        nano_test("decode_stop_large", iters, || {
            black_box(decode_request(black_box(&enc_large_stop)).unwrap());
        }),
        nano_test("encode_remove_small", iters, || {
            black_box(encode_request(black_box(&small_remove)).unwrap());
        }),
        nano_test("decode_remove_small", iters, || {
            black_box(decode_request(black_box(&enc_small_remove)).unwrap());
        }),
        nano_test("encode_remove_large", iters, || {
            black_box(encode_request(black_box(&large_remove)).unwrap());
        }),
        nano_test("decode_remove_large", iters, || {
            black_box(decode_request(black_box(&enc_large_remove)).unwrap());
        }),
        nano_test("encode_list", iters, || {
            black_box(encode_request(black_box(&list_req)).unwrap());
        }),
        nano_test("decode_list", iters, || {
            black_box(decode_request(black_box(&enc_list)).unwrap());
        }),
        nano_test("encode_container_created_small", iters, || {
            black_box(encode_response(black_box(&small_created)).unwrap());
        }),
        nano_test("decode_container_created_small", iters, || {
            black_box(decode_response(black_box(&enc_small_created)).unwrap());
        }),
        nano_test("encode_container_created_large", iters, || {
            black_box(encode_response(black_box(&large_created)).unwrap());
        }),
        nano_test("decode_container_created_large", iters, || {
            black_box(decode_response(black_box(&enc_large_created)).unwrap());
        }),
        nano_test("encode_success_small", iters, || {
            black_box(encode_response(black_box(&small_success)).unwrap());
        }),
        nano_test("decode_success_small", iters, || {
            black_box(decode_response(black_box(&enc_small_success)).unwrap());
        }),
        nano_test("encode_success_large", iters, || {
            black_box(encode_response(black_box(&large_success)).unwrap());
        }),
        nano_test("decode_success_large", iters, || {
            black_box(decode_response(black_box(&enc_large_success)).unwrap());
        }),
        nano_test("encode_error_small", iters, || {
            black_box(encode_response(black_box(&small_error)).unwrap());
        }),
        nano_test("decode_error_small", iters, || {
            black_box(decode_response(black_box(&enc_small_error)).unwrap());
        }),
        nano_test("encode_error_large", iters, || {
            black_box(encode_response(black_box(&large_error)).unwrap());
        }),
        nano_test("decode_error_large", iters, || {
            black_box(decode_response(black_box(&enc_large_error)).unwrap());
        }),
        nano_test("encode_container_list_small", iters, || {
            black_box(encode_response(black_box(&small_list)).unwrap());
        }),
        nano_test("decode_container_list_small", iters, || {
            black_box(decode_response(black_box(&enc_small_list)).unwrap());
        }),
        nano_test("encode_container_list_large", iters, || {
            black_box(encode_response(black_box(&large_list)).unwrap());
        }),
        nano_test("decode_container_list_large", iters, || {
            black_box(decode_response(black_box(&enc_large_list)).unwrap());
        }),
    ];

    let invalid_req: &[u8] = b"{not-json\n";
    let invalid_resp: &[u8] = br#"{"type":"Unknown"}\n"#;
    tests.push(nano_test("decode_invalid_request", iters, || {
        black_box(decode_request(black_box(invalid_req)).is_err());
    }));
    tests.push(nano_test("decode_invalid_response", iters, || {
        black_box(decode_response(black_box(invalid_resp)).is_err());
    }));

    SuiteResult {
        name: "codec".to_string(),
        tests,
    }
}

/// Run the `adapter` micro-benchmark suite.
///
/// Compares direct method calls on concrete mock adapter types against the same
/// calls dispatched through `Arc<dyn Trait>` pointers, plus `Arc::clone` and
/// `downcast_ref` overhead.  10 cases total.  No daemon required.
fn bench_adapter_suite(cfg: &BenchConfig) -> SuiteResult {
    let iters = cfg.iters.max(100);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");

    let layers = vec![PathBuf::from("/layer1")];
    let container_dir = PathBuf::from("/container");
    let resource_cfg = ResourceConfig::default();
    let spawn_cfg = ContainerSpawnConfig {
        rootfs: PathBuf::from("/rootfs"),
        command: "/bin/sh".to_string(),
        args: vec![],
        env: vec![],
        hostname: "test".to_string(),
        cgroup_path: PathBuf::from("/cgroup"),
        capture_output: false,
        hooks: ContainerHooks::default(),
        skip_network_namespace: false,
    };

    let registry_concrete = MockRegistry::new().with_cached_image("alpine", "latest");
    let registry_trait: Arc<dyn ImageRegistry> =
        Arc::new(MockRegistry::new().with_cached_image("alpine", "latest"));
    let fs_concrete = MockFilesystem::new();
    let fs_trait: Arc<dyn FilesystemProvider> = Arc::new(MockFilesystem::new());
    let limiter_concrete = MockLimiter::new();
    let limiter_trait: Arc<dyn ResourceLimiter> = Arc::new(MockLimiter::new());
    let runtime_concrete = MockRuntime::new();
    let runtime_trait: Arc<dyn ContainerRuntime> = Arc::new(MockRuntime::new());
    let arc_for_clone: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());
    let arc_for_downcast: Arc<dyn ImageRegistry> = Arc::new(MockRegistry::new());

    let tests = vec![
        nano_test("registry_direct_has_image", iters, || {
            black_box(registry_concrete.has_image_sync("alpine", "latest"));
        }),
        nano_test("registry_trait_object_has_image", iters, || {
            rt.block_on(async {
                black_box(registry_trait.has_image("alpine", "latest")).await;
            });
        }),
        nano_test("filesystem_direct_setup", iters, || {
            black_box(fs_concrete.setup_rootfs(&layers, &container_dir)).ok();
        }),
        nano_test("filesystem_trait_object_setup", iters, || {
            black_box(fs_trait.setup_rootfs(&layers, &container_dir)).ok();
        }),
        nano_test("limiter_direct_create", iters, || {
            black_box(limiter_concrete.create("container-123", &resource_cfg)).ok();
        }),
        nano_test("limiter_trait_object_create", iters, || {
            black_box(limiter_trait.create("container-123", &resource_cfg)).ok();
        }),
        nano_test("runtime_direct_spawn", iters, || {
            black_box(runtime_concrete.spawn_process_sync(&spawn_cfg)).ok();
        }),
        nano_test("runtime_trait_object_spawn", iters, || {
            rt.block_on(async {
                black_box(runtime_trait.spawn_process(&spawn_cfg).await).ok();
            });
        }),
        nano_test("arc_clone", iters, || {
            black_box(Arc::clone(&arc_for_clone));
        }),
        nano_test("downcast_to_concrete", iters, || {
            black_box(arc_for_downcast.as_any().downcast_ref::<MockRegistry>());
        }),
    ];

    SuiteResult {
        name: "adapter".to_string(),
        tests,
    }
}

/// Print usage information to stdout and return.
fn print_help() {
    println!(
        "minibox-bench\n\nFlags:\n  --iters <N>\n  --cold/--no-cold\n  --warm/--no-warm\n  --dry-run\n  --profile\n  --suite <name>  (pull|run|ps|stop|rm|exec|e2e|codec|adapter|colima)\n  --out-dir <path>"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_serializes() {
        let report = BenchReport::empty();
        let json = serde_json::to_string(&report).unwrap();
        assert!(json.contains("\"metadata\""));
    }

    #[test]
    fn stats_min_avg_p95() {
        let data = vec![10u64, 20, 30, 40, 50];
        let stats = Stats::from_samples(&data);
        assert_eq!(stats.min, 10);
        assert_eq!(stats.avg, 30);
        assert_eq!(stats.p95, 50);
    }

    #[test]
    fn default_iters_is_20() {
        let args = vec!["bench".to_string()];
        let cfg = BenchConfig::from_args(args).unwrap();
        assert_eq!(cfg.iters, 20);
    }

    #[test]
    fn command_runner_captures_exit_status() {
        let true_cmd = if cfg!(target_os = "macos") {
            "/usr/bin/true"
        } else {
            "/bin/true"
        };
        let result = run_cmd(true_cmd, &[]).unwrap();
        assert!(result.success);
    }

    #[test]
    fn suite_has_results() {
        let cfg = BenchConfig::default();
        let results_dir = PathBuf::from("/tmp");
        let timestamp = chrono::Utc::now().to_rfc3339();
        let report = run_suites(&cfg, true, &results_dir, &timestamp).unwrap();
        assert!(!report.suites.is_empty());
    }

    #[test]
    fn report_writes_json() {
        let report = BenchReport::empty();
        let path = "/tmp/bench-report.json";
        write_json(&report, path).unwrap();
        assert!(std::path::Path::new(path).exists());
    }

    #[test]
    fn dry_run_skips_execution() {
        let cfg = BenchConfig {
            dry_run: true,
            ..BenchConfig::default()
        };
        let report = run_benchmark(&cfg).unwrap();
        assert!(!report.suites.is_empty());
    }

    #[test]
    fn planned_suites_default_covers_cli_commands() {
        let suites = planned_suites(&BenchConfig::default());
        for name in ["pull", "run", "ps", "stop", "rm", "exec", "e2e"] {
            assert!(
                suites.iter().any(|suite| suite == name),
                "expected {name} in planned suites"
            );
        }
    }

    #[test]
    fn nano_test_uses_durations_nanos_not_micros() {
        let result = nano_test("test", 5, || {
            std::hint::black_box(1 + 1);
        });
        assert!(
            !result.durations_nanos.is_empty(),
            "durations_nanos must be populated"
        );
        assert!(
            result.durations_micros.is_empty(),
            "durations_micros must be empty for nano tests"
        );
        assert_eq!(result.unit, "nanos");
    }
}
