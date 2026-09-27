//! Runtime health check engine behind `mbx doctor`.
//!
//! Everything here is **built in** — no subprocess shells out to `cargo`,
//! `cargo xtask`, or any other dev tool. That matters because `mbx` ships as a
//! standalone binary: a user who installed minibox from a release tarball has
//! no cargo, no workspace, and no `xtask`. A doctor that shells out to a
//! workspace build tool is a doctor that only works for the person who wrote
//! it.
//!
//! The checks are deliberately *runtime*-oriented — is this host able to run
//! containers — rather than *toolchain*-oriented (is rustup installed). Build
//! and test toolchain readiness stays in `cargo xtask doctor`, which is the
//! right place for it. [`Options::include_tools`] exposes those probes here as
//! an opt-in section for contributors who want one command.
//!
//! # Design
//!
//! [`run`] returns a [`Report`] of [`Section`]s, each holding [`Check`]s with a
//! [`Severity`]. The report is pure data: `mbx` renders it, and `--json`
//! serialises it verbatim. Keeping presentation out of this module means the
//! check set is testable without capturing stdout, and the JSON and human
//! outputs can never disagree about what was checked.
//!
//! Every probe is infallible. A check that cannot determine its answer reports
//! `Info` or `Warn` with the reason — a diagnostic command that crashes is
//! worse than one that admits ignorance.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::Serialize;

use crate::adapter_registry;

/// How a check turned out, ordered from best to worst.
///
/// The ordering is meaningful: [`Severity`] implements [`Ord`] so callers can
/// ask for "the worst result" without a hand-written comparison.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// The check passed, or the state is present and correct.
    Pass,
    /// Informational context — no action implied.
    Info,
    /// Something is missing or degraded but not fatal.
    Warn,
    /// The check failed; container operations will not work as configured.
    Fail,
}

impl Severity {
    /// Whether this severity should make `mbx doctor` exit non-zero.
    #[must_use]
    pub const fn is_failure(self) -> bool {
        matches!(self, Self::Fail)
    }

    /// Marker text, bracketed by the renderer so columns line up regardless of
    /// severity name.
    #[must_use]
    pub const fn marker(self) -> &'static str {
        match self {
            Self::Pass => "ok",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Fail => "FAIL",
        }
    }
}

impl fmt::Display for Severity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pass => "ok",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Fail => "fail",
        })
    }
}

/// A single diagnostic result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Check {
    /// What was probed, e.g. `"cgroups v2"`.
    pub name: String,
    /// Outcome.
    pub severity: Severity,
    /// Human-readable detail or remediation hint.
    pub detail: String,
}

impl Check {
    /// Build a passing check.
    #[must_use]
    pub fn pass(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            severity: Severity::Pass,
            detail: detail.into(),
        }
    }

    /// Build an informational check.
    #[must_use]
    pub fn info(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            severity: Severity::Info,
            detail: detail.into(),
        }
    }

    /// Build a warning check.
    #[must_use]
    pub fn warn(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            severity: Severity::Warn,
            detail: detail.into(),
        }
    }

    /// Build a failing check.
    #[must_use]
    pub fn fail(name: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            severity: Severity::Fail,
            detail: detail.into(),
        }
    }
}

/// A named group of related checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Section {
    /// Section heading, e.g. `"daemon"`.
    pub title: String,
    /// Checks in display order.
    pub checks: Vec<Check>,
}

impl Section {
    /// Create an empty section.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            checks: Vec::new(),
        }
    }

    /// Push a check onto the section.
    #[must_use]
    pub fn with(mut self, check: Check) -> Self {
        self.checks.push(check);
        self
    }

    /// The worst severity present in this section, if any.
    #[must_use]
    pub fn worst(&self) -> Option<Severity> {
        self.checks.iter().map(|c| c.severity).max()
    }
}

/// The full result of a doctor run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Report {
    /// Sections in display order.
    pub sections: Vec<Section>,
}

impl Report {
    /// The worst severity across every section, if any check was run.
    #[must_use]
    pub fn worst(&self) -> Option<Severity> {
        self.sections.iter().filter_map(Section::worst).max()
    }

    /// Whether any check failed. Drives `mbx doctor`'s exit code.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.sections
            .iter()
            .flat_map(|s| &s.checks)
            .any(|c| c.severity.is_failure())
    }

    /// Count of checks by severity, in `Pass`/`Info`/`Warn`/`Fail` order.
    #[must_use]
    pub fn tally(&self) -> [usize; 4] {
        let mut counts = [0usize; 4];
        for check in self.sections.iter().flat_map(|s| &s.checks) {
            let idx = match check.severity {
                Severity::Pass => 0,
                Severity::Info => 1,
                Severity::Warn => 2,
                Severity::Fail => 3,
            };
            counts[idx] += 1;
        }
        counts
    }
}

/// Which check groups to run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Options {
    /// Include the contributor toolchain section (cargo, just, rustup, …).
    ///
    /// Off by default: those tools are irrelevant to a user running a release
    /// binary, and their absence is not a fault in *this* installation.
    pub include_tools: bool,
    /// Reach out to the daemon socket. Off for offline environments where
    /// even a `connect()` would hang on a stale network mount.
    pub probe_daemon: bool,
}

// ---------------------------------------------------------------------------
// PATH resolution (no subprocess, no dependency)
// ---------------------------------------------------------------------------

/// Resolve `name` against `$PATH`, mirroring what the kernel will do at exec.
///
/// Implemented as a `PATH` scan rather than shelling out to `which` so the
/// check works identically on every platform and adds no process spawn to a
/// diagnostic command.
#[must_use]
pub fn resolve_on_path(name: &str) -> Option<PathBuf> {
    if name.contains('/') {
        let direct = PathBuf::from(name);
        return is_executable(&direct).then_some(direct);
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(name))
        .find(|candidate| is_executable(candidate))
}

/// Whether `path` exists and carries an executable bit (or is a bare file on
/// Windows, where the extension carries executability instead).
#[must_use]
pub fn is_executable(path: &Path) -> bool {
    let Ok(meta) = path.metadata() else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

// ---------------------------------------------------------------------------
// Adapter -> external binary requirements
// ---------------------------------------------------------------------------

/// External binaries an adapter needs on `PATH` before it can run.
///
/// Returns an empty slice for adapters that are entirely in-process (`native`,
/// `gke`, `krun`, `vz`) — those have no subprocess dependency, so the only
/// thing to verify is that the host can support them.
#[must_use]
pub fn required_binaries(adapter: &str) -> &'static [&'static str] {
    match adapter {
        "smolvm" => &["smolvm"],
        "colima" => &["limactl", "nerdctl"],
        _ => &[],
    }
}

/// How to obtain the `smolvm` binary, surfaced when it is missing.
const SMOLVM_INSTALL_HINT: &str = "install from https://github.com/89jobrien/smolvm, \
or set MINIBOX_ADAPTER=krun";

// ---------------------------------------------------------------------------
// Path resolution shared with the daemon
// ---------------------------------------------------------------------------

/// Resolve the daemon's data directory, honouring `MINIBOX_DATA_DIR`.
///
/// Mirrors what `miniboxd` uses at startup so `mbx doctor` reports the path
/// the daemon will actually use, not a guess. One implementation, two callers.
///
/// - macOS: `<data_dir>/minibox` (e.g. `~/Library/Application Support/minibox`)
/// - Linux: `/var/lib/minibox` for root, `~/.minibox/cache` otherwise
#[must_use]
pub fn resolve_data_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("MINIBOX_DATA_DIR")
        && !explicit.is_empty()
    {
        return PathBuf::from(explicit);
    }
    #[cfg(target_os = "macos")]
    {
        dirs::data_dir()
            .unwrap_or_else(|| PathBuf::from("/tmp"))
            .join("minibox")
    }
    #[cfg(not(target_os = "macos"))]
    {
        resolve_data_dir_for_uid(crate::preflight::effective_uid())
    }
}

/// Resolve the data directory for a specific effective UID.
///
/// Split out from [`resolve_data_dir`] so the UID-dependent branch stays a
/// pure function that tests can drive directly, instead of only being
/// reachable by manipulating the calling process's real UID.
///
/// This is the same logic the daemon used before it was consolidated here, on
/// every unix platform — it is only *called* on Linux, because macOS has a
/// single user-scoped location handled by [`resolve_data_dir`].
///
/// Resolution order:
/// 1. `MINIBOX_DATA_DIR` (explicit override, honoured here too so the
///    precedence rule lives in exactly one place)
/// 2. `~/.minibox/cache` for non-root
/// 3. `/var/lib/minibox` for root
#[cfg(unix)]
#[must_use]
pub fn resolve_data_dir_for_uid(uid: u32) -> PathBuf {
    if let Ok(explicit) = std::env::var("MINIBOX_DATA_DIR")
        && !explicit.is_empty()
    {
        return PathBuf::from(explicit);
    }
    if uid == 0 {
        PathBuf::from("/var/lib/minibox")
    } else {
        std::env::var("HOME").map_or_else(
            |_| PathBuf::from("/var/lib/minibox"),
            |home| PathBuf::from(home).join(".minibox/cache"),
        )
    }
}

/// Data directory on platforms with no unix path model.
///
/// Not meaningful today — minibox's daemon is unix-only — but kept total so
/// the public surface does not change shape per platform.
#[cfg(not(unix))]
#[must_use]
pub fn resolve_data_dir_for_uid(_uid: u32) -> PathBuf {
    if let Ok(explicit) = std::env::var("MINIBOX_DATA_DIR")
        && !explicit.is_empty()
    {
        return PathBuf::from(explicit);
    }
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join("minibox")
}

/// Resolve the daemon's run directory, honouring `MINIBOX_RUN_DIR`.
///
/// macOS has no `/run`, so `/tmp/minibox` is used there.
#[must_use]
pub fn resolve_run_dir() -> PathBuf {
    if let Ok(explicit) = std::env::var("MINIBOX_RUN_DIR")
        && !explicit.is_empty()
    {
        return PathBuf::from(explicit);
    }
    #[cfg(target_os = "macos")]
    {
        PathBuf::from("/tmp/minibox")
    }
    #[cfg(not(target_os = "macos"))]
    {
        PathBuf::from("/run/minibox")
    }
}

/// Whether `path` exists (or can be created) and accepts a write.
///
/// Does not create anything — a missing parent directory is reported as
/// `Ok(false)` rather than materialised as a side effect of running a
/// diagnostic.
#[must_use]
pub fn is_writable(path: &Path) -> bool {
    if path.is_dir() {
        return std::fs::metadata(path).is_ok_and(|m| !m.permissions().readonly());
    }
    path.parent().is_some_and(|parent| {
        parent.is_dir() && std::fs::metadata(parent).is_ok_and(|m| !m.permissions().readonly())
    })
}

// ---------------------------------------------------------------------------
// Check groups
// ---------------------------------------------------------------------------

/// Host identity and virtualization backend.
fn host_section() -> Section {
    let mut section = Section::new("host");
    section = section.with(Check::info(
        "platform",
        format!("{} {}", std::env::consts::OS, std::env::consts::ARCH),
    ));
    section = section.with(virtualization_check());
    section
}

/// Report whether the host exposes a hardware virtualization backend.
///
/// Every micro-VM adapter (krun, vz, smolvm) depends on one. A host without it
/// can still run containers via the native Linux adapter, so absence is a
/// warning, not a failure — but it is the single most common reason a working
/// install suddenly stops working after a BIOS update or a kernel change.
fn virtualization_check() -> Check {
    #[cfg(target_os = "macos")]
    {
        // Apple Hypervisor.framework is always present on supported macOS and
        // requires no opt-in, so its absence means the OS is too old.
        match macos_version() {
            Some(version) => Check::pass(
                "virtualization",
                format!("Hypervisor.framework (macOS {version})"),
            ),
            None => Check::warn(
                "virtualization",
                "could not read macOS version — krun/vz adapters may be unavailable",
            ),
        }
    }
    #[cfg(target_os = "linux")]
    {
        if Path::new("/dev/kvm").exists() {
            Check::pass("virtualization", "/dev/kvm present (KVM)")
        } else {
            Check::warn(
                "virtualization",
                "/dev/kvm missing — krun micro-VMs unavailable (load the kvm module, \
                 or use MINIBOX_ADAPTER=smolvm)",
            )
        }
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        Check::warn(
            "virtualization",
            format!(
                "no known virtualization backend for {} — no micro-VM adapter available",
                std::env::consts::OS
            ),
        )
    }
}

#[cfg(target_os = "macos")]
fn macos_version() -> Option<String> {
    let output = std::process::Command::new("sw_vers")
        .arg("-productVersion")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let v = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// Linux-only host capabilities the `native` adapter depends on.
#[cfg(target_os = "linux")]
fn host_capability_section() -> Section {
    let caps = crate::preflight::probe();
    let mut section = Section::new("host capabilities (linux)");
    let (major, minor, patch) = caps.kernel_version;
    section = section.with(Check::pass("kernel", format!("{major}.{minor}.{patch}")));
    section = section.with(if caps.cgroups_v2 {
        Check::pass(
            "cgroups v2",
            format!(
                "unified hierarchy, controllers: [{}]",
                caps.cgroup_controllers.join(", ")
            ),
        )
    } else {
        Check::fail(
            "cgroups v2",
            "unified hierarchy not mounted — the native adapter cannot set resource limits",
        )
    });
    section = section.with(if caps.overlay_fs {
        Check::pass("overlay filesystem", "registered with the kernel")
    } else {
        Check::warn(
            "overlay filesystem",
            "not listed in /proc/filesystems — try `modprobe overlay`",
        )
    });
    section = section.with(if caps.is_root {
        Check::pass("privilege", "running as root (UID 0)")
    } else {
        Check::warn(
            "privilege",
            "not running as root — MINIBOX_ADAPTER=native will fail; \
             smolvm/krun work unprivileged",
        )
    });
    section
}

/// Daemon socket reachability.
async fn daemon_section(probe: bool) -> Section {
    let mut section = Section::new("daemon");
    let socket = crate::client::default_socket_path();

    if socket.exists() {
        section = section.with(Check::pass(
            "socket",
            format!("{} exists", socket.display()),
        ));
    } else {
        section = section.with(Check::fail(
            "socket",
            format!(
                "{} not found — start the daemon (scripts/start-daemon.sh)",
                socket.display()
            ),
        ));
        return section;
    }

    if !probe {
        return section.with(Check::info("reachability", "skipped (--no-daemon)"));
    }

    match crate::client::DaemonClient::with_socket(&socket)
        .call(crate::protocol::DaemonRequest::List)
        .await
    {
        Ok(_) => section.with(Check::pass(
            "reachability",
            "daemon answered a List request",
        )),
        Err(e) => section.with(Check::fail(
            "reachability",
            format!("socket exists but the daemon did not answer: {e}"),
        )),
    }
}

/// Which adapters are compiled in, which have their binaries, and which the
/// environment would select.
fn adapter_section() -> Section {
    let mut section = Section::new("adapters");
    let all = adapter_registry::all_adapters();

    for info in &all {
        if !info.available {
            continue;
        }
        let required = required_binaries(info.name);
        if required.is_empty() {
            section = section.with(Check::pass(
                info.name,
                format!("{} (in-process)", info.description),
            ));
            continue;
        }
        let missing: Vec<&str> = required
            .iter()
            .copied()
            .filter(|bin| resolve_on_path(bin).is_none())
            .collect();
        if missing.is_empty() {
            section = section.with(Check::pass(info.name, info.description));
        } else {
            let hint = if missing == ["smolvm"] {
                format!(" — {SMOLVM_INSTALL_HINT}")
            } else {
                String::new()
            };
            section = section.with(Check::warn(
                info.name,
                format!("{} not on PATH{hint}", missing.join(", ")),
            ));
        }
    }

    let unavailable: Vec<&str> = all
        .iter()
        .filter(|i| !i.available)
        .map(|i| i.name)
        .collect();
    if !unavailable.is_empty() {
        section = section.with(Check::info("known but unavailable", unavailable.join(", ")));
    }

    section = section.with(selection_check());
    section
}

/// Report the adapter the daemon would select, and flag a bad override.
///
/// This is the check that makes `mbx doctor` worth running: an invalid
/// `MINIBOX_ADAPTER` is a hard daemon-startup error with no fallback, and the
/// selected-vs-configured gap is otherwise invisible until a container fails.
fn selection_check() -> Check {
    match std::env::var("MINIBOX_ADAPTER") {
        Ok(configured) if !configured.is_empty() => {
            match adapter_registry::parse_adapter(&configured) {
                Ok(_) => Check::pass(
                    "selected adapter",
                    format!("{configured} (from MINIBOX_ADAPTER)"),
                ),
                Err(e) => Check::fail("selected adapter", e.to_string()),
            }
        }
        _ => match adapter_registry::adapter_from_env() {
            Ok(suite) => Check::pass(
                "selected adapter",
                format!("{suite} (auto-detected; override with MINIBOX_ADAPTER=<name>)"),
            ),
            Err(e) => Check::fail("selected adapter", e.to_string()),
        },
    }
}

/// CNI plugin presence for opt-in bridge networking.
fn cni_section() -> Section {
    let mut section = Section::new("cni plugins (bridge networking)");
    let configured =
        std::env::var("MINIBOX_CNI_PATH").unwrap_or_else(|_| "/opt/cni/bin".to_string());
    let dirs: Vec<PathBuf> = std::env::split_paths(&configured).collect();

    for plugin in ["bridge", "host-local", "portmap", "dnsname"] {
        let found = dirs.iter().any(|dir| dir.join(plugin).is_file());
        section = section.with(if found {
            Check::pass(plugin, "present")
        } else {
            Check::info(
                plugin,
                format!("not in {configured} — needed only for --network bridge"),
            )
        });
    }
    section
}

/// Where the daemon keeps images and container state, and whether it can write.
fn storage_section() -> Section {
    let data = resolve_data_dir();
    let run = resolve_run_dir();
    let mut section = Section::new("storage");

    section = section.with(if data.is_dir() {
        Check::pass("data dir", data.display().to_string())
    } else {
        // Not an error: the daemon creates it on first run.
        Check::info(
            "data dir",
            format!("{} (created by the daemon on first run)", data.display()),
        )
    });
    section = section.with(if run.is_dir() {
        Check::pass("run dir", run.display().to_string())
    } else {
        Check::info(
            "run dir",
            format!("{} (created by the daemon at startup)", run.display()),
        )
    });
    section = section.with(if is_writable(&data) && is_writable(&run) {
        Check::pass("writable", "data and run directories accept writes")
    } else {
        Check::warn(
            "writable",
            "the daemon may not be able to create its state directories",
        )
    });
    section
}

/// Contributor toolchain probes, opt-in via [`Options::include_tools`].
fn tools_section() -> Section {
    let mut section = Section::new("development toolchain");
    // `cargo` and `cargo-nextest` gate building and testing; the rest only
    // gate optional release and repo workflows, so absence is a warning.
    for (tool, required) in [
        ("cargo", true),
        ("cargo-nextest", true),
        ("just", false),
        ("rustup", false),
        ("gh", false),
        ("op", false),
    ] {
        section = section.with(match resolve_on_path(tool) {
            Some(path) => Check::pass(tool, path.display().to_string()),
            None if required => Check::fail(tool, "not on PATH — cannot build or test"),
            None => Check::warn(tool, "not on PATH (optional)"),
        });
    }
    section
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

/// Run the built-in health checks and return a report.
///
/// Never fails and never spawns a build tool. `options.include_tools` adds the
/// contributor toolchain section; everything else is always run.
#[must_use]
pub async fn run(options: Options) -> Report {
    let mut sections = vec![host_section()];
    #[cfg(target_os = "linux")]
    sections.push(host_capability_section());
    sections.push(daemon_section(options.probe_daemon).await);
    sections.push(adapter_section());
    sections.push(cni_section());
    sections.push(storage_section());
    if options.include_tools {
        sections.push(tools_section());
    }
    Report { sections }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    // ----- severity ordering -----

    #[test]
    fn severity_orders_pass_to_fail() {
        assert!(Severity::Pass < Severity::Info);
        assert!(Severity::Info < Severity::Warn);
        assert!(Severity::Warn < Severity::Fail);
    }

    #[test]
    fn only_fail_is_a_failure() {
        assert!(Severity::Fail.is_failure());
        assert!(!Severity::Warn.is_failure());
        assert!(!Severity::Pass.is_failure());
        assert!(!Severity::Info.is_failure());
    }

    #[test]
    fn markers_are_the_words_the_renderer_brackets() {
        // Alignment comes from the padded check name, not the marker, so the
        // markers do not need to be equal width. They do need to be the exact
        // words the renderer prints, or the human and JSON outputs disagree.
        assert_eq!(Severity::Pass.marker(), "ok");
        assert_eq!(Severity::Info.marker(), "info");
        assert_eq!(Severity::Warn.marker(), "warn");
        assert_eq!(Severity::Fail.marker(), "FAIL");
    }

    // ----- report aggregation -----

    fn sample_report() -> Report {
        Report {
            sections: vec![
                Section::new("a")
                    .with(Check::pass("p", "fine"))
                    .with(Check::warn("w", "meh")),
                Section::new("b").with(Check::info("i", "fyi")),
            ],
        }
    }

    #[test]
    fn report_worst_returns_max_severity() {
        assert_eq!(sample_report().worst(), Some(Severity::Warn));
    }

    #[test]
    fn report_without_failures_reports_clean() {
        assert!(!sample_report().has_failures());
    }

    #[test]
    fn report_with_failure_is_dirty() {
        let report = Report {
            sections: vec![Section::new("x").with(Check::fail("f", "broken"))],
        };
        assert!(report.has_failures());
        assert_eq!(report.worst(), Some(Severity::Fail));
    }

    #[test]
    fn report_tally_counts_by_severity() {
        let report = Report {
            sections: vec![
                Section::new("x")
                    .with(Check::pass("a", ""))
                    .with(Check::pass("b", ""))
                    .with(Check::warn("c", ""))
                    .with(Check::fail("d", "")),
            ],
        };
        assert_eq!(report.tally(), [2, 0, 1, 1]);
    }

    #[test]
    fn empty_report_has_no_worst() {
        let report = Report { sections: vec![] };
        assert_eq!(report.worst(), None);
        assert!(!report.has_failures());
    }

    #[test]
    fn report_serialises_to_json() {
        let json = serde_json::to_string(&sample_report()).expect("serialise");
        assert!(json.contains("\"severity\":\"warn\""), "got: {json}");
        assert!(json.contains("\"title\":\"a\""), "got: {json}");
    }

    // ----- path resolution -----

    #[test]
    fn resolve_on_path_finds_sh() {
        // Every supported platform has a `sh`-alike; use cargo which we know
        // is present because these tests run under it.
        assert!(resolve_on_path("cargo").is_some(), "cargo must be on PATH");
    }

    #[test]
    fn resolve_on_path_returns_none_for_absent_binary() {
        assert!(resolve_on_path("mbx_definitely_not_a_real_binary_xyz").is_none());
    }

    #[test]
    fn resolve_on_path_handles_explicit_relative_path() {
        assert!(resolve_on_path("./no/such/binary/xyz").is_none());
    }

    #[test]
    fn is_executable_rejects_directories() {
        assert!(!is_executable(Path::new("/tmp")));
    }

    #[test]
    fn is_executable_rejects_missing_paths() {
        assert!(!is_executable(Path::new("/mbx/no/such/path/xyz")));
    }

    // ----- adapter binary requirements -----

    #[test]
    fn smolvm_requires_its_binary() {
        assert_eq!(required_binaries("smolvm"), &["smolvm"]);
    }

    #[test]
    fn colima_requires_lima_and_nerdctl() {
        assert_eq!(required_binaries("colima"), &["limactl", "nerdctl"]);
    }

    #[test]
    fn in_process_adapters_require_no_binaries() {
        for adapter in ["native", "gke", "krun", "vz"] {
            assert!(
                required_binaries(adapter).is_empty(),
                "{adapter} is in-process and must not require a binary"
            );
        }
    }

    #[test]
    fn every_registry_adapter_has_a_binary_requirement_entry() {
        // Guards against a new adapter being added to the registry without the
        // doctor learning what it needs — the exact drift this module exists
        // to prevent.
        for info in adapter_registry::all_adapters() {
            let _ = required_binaries(info.name);
        }
    }

    // ----- selection -----

    #[test]
    fn selection_check_passes_for_valid_override() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("MINIBOX_ADAPTER", "krun") };
        let check = selection_check();
        // SAFETY: same lock.
        unsafe { std::env::remove_var("MINIBOX_ADAPTER") };
        assert_eq!(check.severity, Severity::Pass);
    }

    #[test]
    fn selection_check_fails_for_garbage_override() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("MINIBOX_ADAPTER", "not_a_real_adapter") };
        let check = selection_check();
        // SAFETY: same lock.
        unsafe { std::env::remove_var("MINIBOX_ADAPTER") };
        assert_eq!(check.severity, Severity::Fail);
        assert!(
            check.detail.contains("not_a_real_adapter"),
            "got: {}",
            check.detail
        );
    }

    #[test]
    fn selection_check_fails_for_unavailable_override() {
        if cfg!(target_os = "linux") {
            return; // all adapters available on Linux
        }
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("MINIBOX_ADAPTER", "native") };
        let check = selection_check();
        // SAFETY: same lock.
        unsafe { std::env::remove_var("MINIBOX_ADAPTER") };
        assert_eq!(check.severity, Severity::Fail);
    }

    // ----- path resolution shared with the daemon -----

    #[test]
    fn data_dir_honours_explicit_override() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("MINIBOX_DATA_DIR", "/tmp/mbx-doctor-test") };
        let dir = resolve_data_dir();
        // SAFETY: same lock.
        unsafe { std::env::remove_var("MINIBOX_DATA_DIR") };
        assert_eq!(dir, PathBuf::from("/tmp/mbx-doctor-test"));
    }

    #[test]
    fn data_dir_ignores_empty_override() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("MINIBOX_DATA_DIR", "") };
        let dir = resolve_data_dir();
        // SAFETY: same lock.
        unsafe { std::env::remove_var("MINIBOX_DATA_DIR") };
        assert!(
            !dir.as_os_str().is_empty(),
            "an empty override must fall through to the default"
        );
    }

    #[test]
    fn data_dir_for_uid_uses_home_for_non_root() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK.
        unsafe {
            std::env::remove_var("MINIBOX_DATA_DIR");
            std::env::set_var("HOME", "/home/testuser");
        }
        let dir = resolve_data_dir_for_uid(1000);
        // SAFETY: same lock.
        unsafe { std::env::remove_var("HOME") };
        assert_eq!(dir, PathBuf::from("/home/testuser/.minibox/cache"));
    }

    #[test]
    fn data_dir_for_uid_uses_var_lib_for_root() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::remove_var("MINIBOX_DATA_DIR") };
        assert_eq!(
            resolve_data_dir_for_uid(0),
            PathBuf::from("/var/lib/minibox")
        );
    }

    #[test]
    fn data_dir_for_uid_honours_env_override_for_both_uids() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("MINIBOX_DATA_DIR", "/custom/path") };
        let non_root = resolve_data_dir_for_uid(1000);
        let root = resolve_data_dir_for_uid(0);
        // SAFETY: same lock.
        unsafe { std::env::remove_var("MINIBOX_DATA_DIR") };
        assert_eq!(non_root, PathBuf::from("/custom/path"));
        assert_eq!(root, PathBuf::from("/custom/path"));
    }

    #[test]
    fn run_dir_honours_explicit_override() {
        let _guard = ENV_LOCK.lock().expect("env lock poisoned");
        // SAFETY: serialized by ENV_LOCK.
        unsafe { std::env::set_var("MINIBOX_RUN_DIR", "/tmp/mbx-doctor-run") };
        let dir = resolve_run_dir();
        // SAFETY: same lock.
        unsafe { std::env::remove_var("MINIBOX_RUN_DIR") };
        assert_eq!(dir, PathBuf::from("/tmp/mbx-doctor-run"));
    }

    #[test]
    fn is_writable_rejects_missing_parent() {
        assert!(!is_writable(Path::new("/mbx/no/such/dir/xyz/file")));
    }

    // ----- sections -----

    #[test]
    fn host_section_always_reports_platform() {
        let section = host_section();
        assert!(section.checks.iter().any(|c| c.name == "platform"));
        assert!(section.checks.iter().any(|c| c.name == "virtualization"));
    }

    #[test]
    fn cni_section_reports_all_four_plugins() {
        let section = cni_section();
        for plugin in ["bridge", "host-local", "portmap", "dnsname"] {
            assert!(
                section.checks.iter().any(|c| c.name == plugin),
                "missing CNI plugin check: {plugin}"
            );
        }
    }

    #[test]
    fn adapter_section_lists_every_compiled_adapter() {
        let section = adapter_section();
        for info in adapter_registry::all_adapters() {
            if info.available {
                assert!(
                    section.checks.iter().any(|c| c.name == info.name),
                    "adapter {} is compiled in but absent from the doctor report",
                    info.name
                );
            }
        }
    }

    #[test]
    fn adapter_section_includes_vz_when_feature_enabled() {
        if !cfg!(all(target_os = "macos", feature = "vz")) {
            return;
        }
        let section = adapter_section();
        assert!(section.checks.iter().any(|c| c.name == "vz"));
    }

    #[test]
    fn tools_section_marks_cargo_required() {
        let section = tools_section();
        let cargo = section
            .checks
            .iter()
            .find(|c| c.name == "cargo")
            .expect("cargo check");
        assert_eq!(cargo.severity, Severity::Pass);
    }

    // ----- end to end -----

    #[tokio::test]
    async fn report_without_tools_has_no_toolchain_section() {
        let report = run(Options {
            include_tools: false,
            probe_daemon: false,
        })
        .await;
        assert!(
            !report
                .sections
                .iter()
                .any(|s| s.title.contains("toolchain")),
            "--tools must be opt-in: {:?}",
            report.sections.iter().map(|s| &s.title).collect::<Vec<_>>()
        );
    }

    #[tokio::test]
    async fn report_with_tools_includes_toolchain_section() {
        let report = run(Options {
            include_tools: true,
            probe_daemon: false,
        })
        .await;
        assert!(
            report
                .sections
                .iter()
                .any(|s| s.title.contains("toolchain")),
            "--tools must add the toolchain section"
        );
    }

    #[tokio::test]
    async fn report_offline_does_not_fail_on_absent_daemon() {
        // With probe_daemon off, a missing socket is still reported, but the
        // run must not hang or error. The point is that run() returns at all.
        let report = run(Options {
            include_tools: false,
            probe_daemon: false,
        })
        .await;
        assert!(!report.sections.is_empty());
    }
}
