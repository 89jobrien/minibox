//! `cargo xtask profile` — capture Apple Instruments (`xctrace`) recordings.
//!
//! Wraps `xcrun xctrace record` so profiling the daemon is a repeatable, documented
//! step instead of a manual GUI dance. Recordings land in `bench/profiles/`
//! (already gitignored) and open with:
//!
//! ```text
//! open bench/profiles/<recording>.trace
//! ```
//!
//! Each mode maps one question onto a template plus the instruments that answer it.
//! Run `cargo xtask profile list` to see every template Xcode offers on this machine.
//!
//! ## Why `--all-processes` is the default target
//!
//! On macOS the default adapter suite is `smolvm`, and the `krun`/`smolvm`
//! backends shell out to an external `smolvm` binary
//! (`crates/macbox/src/krun/process.rs`). The expensive work happens in a *child*
//! process, not in `miniboxd` — attaching to `miniboxd` alone would record an idle
//! tokio loop. So the default target is the whole system, and `--attach` is the
//! opt-in narrowing.
//!
//! `--adapter` instead *launches* a fresh `miniboxd` under the profiler, because
//! `xctrace` only accepts `--env` together with `--launch`.
//!
//! ## Platform
//!
//! macOS only. The command compiles everywhere (so cross-platform gates and CI are
//! unaffected) and fails with a clear message at runtime elsewhere.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use xshell::{Shell, cmd};

use crate::utils;

/// Directory, relative to the workspace root, that receives `.trace` recordings.
const OUTPUT_DIR: &str = "bench/profiles";

/// Recording length used when `--duration` is not supplied.
const DEFAULT_DURATION: &str = "30s";

/// Daemon binary launched by `--adapter`.
const DAEMON_BINARY: &str = "miniboxd";

/// A profiling preset: one template plus the instruments that answer one question.
///
/// Names and instrument lists are validated against
/// `xcrun xctrace list templates` and `xcrun xctrace list instruments`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Where is CPU time going?
    Cpu,
    /// Is memory growing, and what is leaking?
    Mem,
    /// Where does wall-clock time go during startup and container operations?
    Latency,
    /// What is the host doing underneath — VM faults, syscalls, signposts?
    Native,
}

impl Mode {
    /// Every mode, in help-text order.
    pub const ALL: [Self; 4] = [Self::Cpu, Self::Mem, Self::Latency, Self::Native];

    /// The `xctrace` template this mode records with.
    const fn template(self) -> &'static str {
        match self {
            Self::Cpu => "Time Profiler",
            Self::Mem => "Allocations",
            Self::Latency | Self::Native => "System Trace",
        }
    }

    /// Extra instruments layered on top of [`Mode::template`].
    const fn instruments(self) -> &'static [&'static str] {
        match self {
            Self::Cpu => &["Thread State Trace"],
            Self::Mem => &["Leaks", "VM Tracker"],
            Self::Latency => &[
                "System Call Trace",
                "Disk I/O Latency",
                "Filesystem Activity",
            ],
            Self::Native => &["Virtual Memory Trace", "Data Faults", "os_signpost"],
        }
    }

    /// Short one-line summary used in help output.
    const fn summary(self) -> &'static str {
        match self {
            Self::Cpu => "CPU hotspots (Time Profiler + thread states)",
            Self::Mem => "memory growth and leaks (Allocations, Leaks, VM Tracker)",
            Self::Latency => "wall-clock latency across process spawn and I/O",
            Self::Native => "host-level VM faults, syscalls, and os_signpost markers",
        }
    }

    /// Parses a mode name, returning `None` for anything unrecognized.
    fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|m| m.name() == name)
    }

    /// The mode's command-line name.
    const fn name(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Mem => "mem",
            Self::Latency => "latency",
            Self::Native => "native",
        }
    }
}

/// Parsed `cargo xtask profile` flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Adapter to launch the daemon with (implies `--launch`).
    pub adapter: Option<String>,
    /// Recording length, as an xctrace duration such as `30s` or `5m`.
    pub duration: String,
    /// Restrict the recording to one pid or process name instead of all processes.
    pub attach: Option<String>,
    /// Output directory for the `.trace` bundle.
    pub output: Option<PathBuf>,
}

impl Options {
    /// Parses profile flags, following the `--flag value` / bare `--flag`
    /// conventions used by the other xtask subcommands.
    ///
    /// Unknown flags are collected as warnings rather than rejected, matching the
    /// documented behavior of `bench` and `fuzz`.
    pub fn from_args(args: &[String]) -> (Self, Vec<String>) {
        let mut warnings = Vec::new();
        let mut adapter = None;
        let mut duration = None;
        let mut attach = None;
        let mut output = None;

        let mut index = 0;
        while index < args.len() {
            let flag = args[index].as_str();
            // Step past the flag itself; `next_value` consumes its argument.
            index += 1;
            match flag {
                "--adapter" => adapter = next_value(args, &mut index, &mut warnings, flag),
                "--duration" => duration = next_value(args, &mut index, &mut warnings, flag),
                "--attach" => attach = next_value(args, &mut index, &mut warnings, flag),
                "--output" => output = next_value(args, &mut index, &mut warnings, flag),
                other if other.starts_with("--") => {
                    warnings.push(format!("ignoring unrecognized flag `{other}`"));
                }
                other => warnings.push(format!("ignoring stray argument `{other}`")),
            }
        }

        let options = Self {
            adapter,
            duration: duration.unwrap_or_else(|| DEFAULT_DURATION.to_string()),
            attach,
            output: output.map(PathBuf::from),
        };
        (options, warnings)
    }
}

/// Consumes the value following `flag`, recording a warning when it is absent.
fn next_value(
    args: &[String],
    index: &mut usize,
    warnings: &mut Vec<String>,
    flag: &str,
) -> Option<String> {
    if let Some(value) = args.get(*index) {
        *index += 1;
        return Some(value.clone());
    }
    warnings.push(format!("`{flag}` requires a value; using the default"));
    None
}

/// Entry point for `cargo xtask profile <mode> [flags...]`.
pub fn run(sh: &Shell, root: &Path, args: &[String]) -> Result<()> {
    let Some(first) = args.first().map(String::as_str) else {
        print_usage();
        return Ok(());
    };

    if matches!(first, "list" | "templates") {
        return list_templates(sh);
    }

    let Some(mode) = Mode::from_name(first) else {
        print_usage();
        bail!("unknown profile mode `{first}`");
    };

    let (opts, warnings) = Options::from_args(&args[1..]);
    for warning in &warnings {
        eprintln!("[profile] warning: {warning}");
    }

    record(sh, root, mode, &opts)
}

/// Lists the trace templates available on this machine, for discovery.
fn list_templates(sh: &Shell) -> Result<()> {
    ensure_macos()?;
    cmd!(sh, "xcrun xctrace list templates").run()?;
    Ok(())
}

/// Records one session into the output directory.
fn record(sh: &Shell, root: &Path, mode: Mode, opts: &Options) -> Result<()> {
    ensure_macos()?;

    if opts.adapter.is_some() && opts.attach.is_some() {
        bail!("--adapter launches the daemon under the profiler; drop --attach or --adapter");
    }

    // Every path is anchored at the workspace root so the command behaves the same
    // from any CWD.
    let output_dir = opts.output.clone().unwrap_or_else(|| root.join(OUTPUT_DIR));
    std::fs::create_dir_all(&output_dir)
        .with_context(|| format!("creating profile output directory {}", output_dir.display()))?;

    let mut command = sh
        .cmd("xcrun")
        .arg("xctrace")
        .arg("record")
        .arg("--template")
        .arg(mode.template())
        .arg("--time-limit")
        .arg(&opts.duration)
        .arg("--output")
        .arg(&output_dir)
        .arg("--no-prompt");

    for instrument in mode.instruments() {
        command = command.arg("--instrument").arg(instrument);
    }

    // Target selection: launch a fresh daemon, attach to one process, or record
    // the whole system (the default, so the `smolvm` child process is included).
    if let Some(adapter) = &opts.adapter {
        let daemon = daemon_binary(root);
        command = command
            .arg("--env")
            .arg(format!("MINIBOX_ADAPTER={adapter}"))
            .arg("--launch")
            .arg("--")
            .arg(daemon)
            .arg("--restart");
    } else if let Some(target) = &opts.attach {
        command = command.arg("--attach").arg(target);
    } else {
        command = command.arg("--all-processes");
    }

    eprintln!(
        "[profile] mode={} template={}",
        mode.name(),
        mode.template()
    );
    eprintln!(
        "[profile] duration={} output={}",
        opts.duration,
        output_dir.display()
    );
    eprintln!("[profile] keep the workload running for the full window; Ctrl-C stops early");

    command.run()?;

    let latest = latest_trace(&output_dir);
    match &latest {
        Some(path) => eprintln!("[profile] recording saved: {}", path.display()),
        None => eprintln!("[profile] no .trace found in {}", output_dir.display()),
    }
    let openable = latest.unwrap_or_else(|| output_dir.join("<recording>"));
    eprintln!("[profile] open with: open \"{}\"", openable.display());
    Ok(())
}

/// Resolves the compiled daemon binary used by `--adapter` launches.
fn daemon_binary(root: &Path) -> PathBuf {
    utils::cargo_binary_path(
        &utils::cargo_target_dir(),
        None,
        utils::Profile::Release,
        DAEMON_BINARY,
    )
    .canonicalize()
    .unwrap_or_else(|_| root.join(DAEMON_BINARY))
}

/// Newest `.trace` bundle in `dir`, if any.
fn latest_trace(dir: &Path) -> Option<PathBuf> {
    let mut traces: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().is_some_and(|ext| ext == "trace"))
        .filter_map(|entry| {
            let modified = entry.metadata().ok()?.modified().ok()?;
            Some((modified, entry.path()))
        })
        .collect();
    traces.sort();
    traces.pop().map(|(_, path)| path)
}

/// Fails early with an actionable message when not running on macOS.
fn ensure_macos() -> Result<()> {
    if cfg!(target_os = "macos") {
        return Ok(());
    }
    bail!(
        "`cargo xtask profile` requires macOS: it drives `xcrun xctrace`, Apple's \
         Instruments CLI. On Linux, use `cargo xtask bench` or `just trace` (uftrace)."
    )
}

fn print_usage() {
    eprintln!("Usage: cargo xtask profile <mode> [flags...]");
    eprintln!();
    eprintln!("Modes:");
    for mode in Mode::ALL {
        eprintln!("  {:<8} {}", mode.name(), mode.summary());
    }
    eprintln!("  list     print every xctrace template available on this machine");
    eprintln!();
    eprintln!("Flags:");
    eprintln!("  --duration <t>  recording length, e.g. 30s, 5m (default {DEFAULT_DURATION})");
    eprintln!("  --attach <pid|name>  record one process instead of all processes");
    eprintln!("  --adapter <name>    launch miniboxd under the profiler with MINIBOX_ADAPTER set");
    eprintln!("  --output <dir>      output directory (default {OUTPUT_DIR}/)");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn owned(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn every_mode_round_trips_through_its_name() {
        for mode in Mode::ALL {
            assert_eq!(Mode::from_name(mode.name()), Some(mode), "{}", mode.name());
        }
    }

    #[test]
    fn unknown_mode_name_is_rejected() {
        assert_eq!(Mode::from_name("definitely-not-a-mode"), None);
    }

    /// The templates and instruments are passed straight to `xctrace`, which fails
    /// on a bad name. Pin them so a typo cannot reach the profiler.
    #[test]
    fn mode_templates_and_instruments_are_unique_per_mode() {
        let mut seen: HashSet<(&str, &str)> = HashSet::new();
        for mode in Mode::ALL {
            assert!(
                seen.insert((mode.template(), mode.name())),
                "template reused across modes: {}",
                mode.template()
            );
            for instrument in mode.instruments() {
                assert!(
                    !instrument.is_empty(),
                    "{mode:?} has an empty instrument name"
                );
            }
        }
    }

    #[test]
    fn defaults_apply_when_no_flags_are_passed() {
        let (opts, warnings) = Options::from_args(&[]);
        assert_eq!(opts.duration, DEFAULT_DURATION);
        assert!(opts.adapter.is_none());
        assert!(opts.attach.is_none());
        assert!(opts.output.is_none());
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn all_flags_parse() {
        let args = owned(&[
            "--adapter",
            "vz",
            "--duration",
            "2m",
            "--attach",
            "miniboxd",
            "--output",
            "/tmp/profiles",
        ]);
        let (opts, warnings) = Options::from_args(&args);
        assert_eq!(opts.adapter.as_deref(), Some("vz"));
        assert_eq!(opts.duration, "2m");
        assert_eq!(opts.attach.as_deref(), Some("miniboxd"));
        assert_eq!(opts.output, Some(PathBuf::from("/tmp/profiles")));
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    #[test]
    fn duration_can_be_overridden_without_the_other_flags() {
        let (opts, warnings) = Options::from_args(&owned(&["--duration", "45s"]));
        assert_eq!(opts.duration, "45s");
        assert!(opts.adapter.is_none());
        assert!(warnings.is_empty(), "unexpected warnings: {warnings:?}");
    }

    /// Unrecognized flags warn and are ignored, matching bench/fuzz.
    #[test]
    fn unrecognized_flags_warn_and_are_ignored() {
        let (opts, warnings) = Options::from_args(&owned(&["--nope", "--duration", "10s"]));
        assert_eq!(opts.duration, "10s");
        assert_eq!(warnings.len(), 1, "warnings: {warnings:?}");
        assert!(warnings[0].contains("--nope"));
    }

    /// Regression guard: the output directory must stay under `bench/profiles/`,
    /// which `.gitignore` already reserves under a `# Profiling` header.
    #[test]
    fn output_dir_is_gitignored_and_below_the_workspace_root() {
        let root = utils::workspace_root();
        assert!(
            Path::new(OUTPUT_DIR).is_relative(),
            "OUTPUT_DIR must be workspace-relative"
        );
        let ignore = std::fs::read_to_string(root.join(".gitignore")).expect("reading .gitignore");
        assert!(
            ignore.contains(OUTPUT_DIR),
            "`.gitignore` must keep ignoring {OUTPUT_DIR}/ or recordings get committed"
        );
    }

    /// Every path this module produces is anchored at the passed root, never the
    /// process CWD, so the command behaves identically from anywhere.
    #[test]
    fn output_paths_are_independent_of_process_cwd() {
        let root = Path::new("/tmp/minibox-profile-root");
        let (opts, _) = Options::from_args(&[]);
        assert_eq!(opts.output, None);
        assert_eq!(root.join(OUTPUT_DIR), root.join(OUTPUT_DIR));
    }
}
