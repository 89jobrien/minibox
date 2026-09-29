//! `cargo xtask fuzz` — run libFuzzer fuzz targets against the protocol codec.
//!
//! Requires a nightly toolchain (`rustup toolchain install nightly`).
//!
//! Targets are discovered from the standalone cargo-fuzz manifest rather than a
//! hardcoded list, and every path is anchored at the workspace root, so the
//! command behaves the same from any CWD.
//!
//! Usage:
//!   cargo xtask fuzz                         # every declared target, 60 s each
//!   cargo xtask fuzz --time 300              # 5 min each
//!   cargo xtask fuzz --target `fuzz_decode_request`
//!   cargo xtask fuzz --smoke                 # bounded run for scheduled CI

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::path::{Path, PathBuf};
use xshell::{Shell, cmd};

/// Root-relative directory holding the standalone cargo-fuzz crate.
///
/// The fuzz crate declares its own `[workspace]`, so it is deliberately *not* a
/// member of the main workspace and cannot be reached through `cargo metadata`
/// on the root manifest.
const FUZZ_CRATE_DIR: &str = "fuzz";

/// Manifest file name inside `FUZZ_CRATE_DIR`.
const FUZZ_MANIFEST_FILE: &str = "Cargo.toml";

/// cargo-fuzz's naming convention for libFuzzer target binaries.
const TARGET_PREFIX: &str = "fuzz_";

/// Per-target budget for a normal local run.
const DEFAULT_TIME_SECS: u64 = 60;

/// Per-target budget for `--smoke`, sized for a scheduled CI job.
const SMOKE_TIME_SECS: u64 = 5;

/// Parsed `cargo xtask fuzz` flags.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Per-target fuzz budget, in seconds.
    pub time_secs: u64,
    /// Restrict the run to a single target; `None` runs every discovered target.
    pub target: Option<String>,
    /// Whether the bounded scheduled-CI mode was requested.
    pub smoke: bool,
}

impl Options {
    /// Parses runner flags, following the same `--flag value` / bare `--flag`
    /// conventions as the other xtask subcommands.
    pub fn from_args(args: &[String]) -> Self {
        let explicit_time: Option<u64> = args
            .windows(2)
            .find(|w| w[0] == "--time")
            .and_then(|w| w[1].parse().ok());
        let smoke = args.iter().any(|a| a == "--smoke");
        let fallback = if smoke {
            SMOKE_TIME_SECS
        } else {
            DEFAULT_TIME_SECS
        };
        Self {
            time_secs: explicit_time.unwrap_or(fallback),
            target: args
                .windows(2)
                .find(|w| w[0] == "--target")
                .map(|w| w[1].clone()),
            smoke,
        }
    }
}

/// The subset of a `[[bin]]` entry needed to tell a libFuzzer target from an
/// ordinary binary in the fuzz crate.
#[derive(Debug, Clone, Deserialize)]
struct FuzzBin {
    /// Target name, as passed to `cargo fuzz run <name>`.
    name: String,
    /// Cargo `test` key. `Some(true)` means the bin is a libtest binary, which
    /// libFuzzer cannot drive.
    #[serde(default)]
    test: Option<bool>,
    /// Cargo `harness` key. `Some(true)` means the bin uses the libtest harness.
    #[serde(default)]
    harness: Option<bool>,
}

impl FuzzBin {
    /// Whether this `[[bin]]` is a libFuzzer target.
    ///
    /// cargo-fuzz requires the `fuzz_` name prefix and emits targets harness-less
    /// (`test = false`, `harness = false`). A `[[bin]]` without the prefix is an
    /// ordinary binary; a prefix-named bin that opts into a test harness is a
    /// test fixture, not a target.
    fn is_fuzz_target(&self) -> bool {
        self.name.starts_with(TARGET_PREFIX)
            && self.test != Some(true)
            && self.harness != Some(true)
    }
}

/// The fuzz crate manifest, reduced to its binary targets.
#[derive(Debug, Default, Deserialize)]
struct FuzzManifest {
    /// Every `[[bin]]` entry the fuzz crate declares.
    #[serde(default)]
    bin: Vec<FuzzBin>,
}

/// Root-relative path of the standalone cargo-fuzz manifest.
pub fn fuzz_manifest_path(root: &Path) -> PathBuf {
    root.join(FUZZ_CRATE_DIR).join(FUZZ_MANIFEST_FILE)
}

/// Discover every libFuzzer target declared by the fuzz manifest, in manifest
/// order.
pub fn discover_targets(root: &Path) -> Result<Vec<String>> {
    let manifest_path = fuzz_manifest_path(root);
    let source = std::fs::read_to_string(&manifest_path)
        .with_context(|| format!("read fuzz manifest {}", manifest_path.display()))?;
    let manifest: FuzzManifest = toml::from_str(&source).context("parse fuzz manifest as TOML")?;
    Ok(manifest
        .bin
        .into_iter()
        .filter(FuzzBin::is_fuzz_target)
        .map(|bin| bin.name)
        .collect())
}

/// Every path one `cargo fuzz run` invocation needs, anchored at the workspace
/// root rather than the process working directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TargetPaths {
    /// The standalone fuzz manifest to build.
    pub manifest: PathBuf,
    /// Writable corpus directory; the fuzzer writes new inputs here.
    pub corpus: PathBuf,
    /// Committed seed inputs.
    pub seeds: PathBuf,
}

/// Resolve the fuzz-crate layout for `target`, anchored at `root`.
///
/// Anchoring at the root (rather than the CWD) keeps `cargo xtask fuzz` correct
/// when invoked from a subdirectory or with `--manifest-path`.
pub fn target_paths(root: &Path, target: &str) -> TargetPaths {
    let crate_root = root.join(FUZZ_CRATE_DIR);
    TargetPaths {
        manifest: crate_root.join(FUZZ_MANIFEST_FILE),
        corpus: crate_root.join("corpus").join(target),
        seeds: crate_root.join("seeds").join(target),
    }
}

/// Narrow the discovered targets to the ones this invocation should run.
fn select_targets(requested: Option<&str>, discovered: &[String]) -> Result<Vec<String>> {
    match requested {
        Some(name) if !discovered.iter().any(|d| d == name) => bail!(
            "unknown fuzz target {name:?}. Available: {}",
            discovered.join(", ")
        ),
        Some(name) => Ok(vec![name.to_owned()]),
        None => Ok(discovered.to_vec()),
    }
}

/// The complete argument vector for a single `cargo fuzz run` invocation.
fn fuzz_args(paths: &TargetPaths, target: &str, time_secs: u64) -> Vec<String> {
    vec![
        "+nightly".to_owned(),
        "fuzz".to_owned(),
        "run".to_owned(),
        target.to_owned(),
        "--manifest-path".to_owned(),
        paths.manifest.display().to_string(),
        paths.corpus.display().to_string(),
        paths.seeds.display().to_string(),
        "--".to_owned(),
        format!("-max_total_time={time_secs}"),
    ]
}

/// Run one libFuzzer target. The single place a `cargo fuzz run` is built.
fn run_target(sh: &Shell, paths: &TargetPaths, target: &str, time_secs: u64) -> Result<()> {
    sh.cmd("cargo")
        .args(fuzz_args(paths, target, time_secs))
        .run()
        .with_context(|| format!("fuzz target {target} crashed — check fuzz/artifacts/"))
}

/// Runs the requested fuzz target or the complete fuzz suite.
pub fn fuzz(sh: &Shell, root: &Path) -> Result<()> {
    let args: Vec<String> = std::env::args().skip(2).collect();
    let options = Options::from_args(&args);

    let discovered = discover_targets(root)?;
    if discovered.is_empty() {
        bail!(
            "no libFuzzer targets declared in {}",
            fuzz_manifest_path(root).display()
        );
    }
    let targets = select_targets(options.target.as_deref(), &discovered)?;

    // Verify nightly is available.
    cmd!(sh, "rustup run nightly cargo --version")
        .quiet()
        .run()
        .context("nightly toolchain not found; run: rustup toolchain install nightly")?;

    let budget = options.time_secs;
    if options.smoke {
        eprintln!(
            "fuzz: smoke mode — {budget}s per target across {} target(s)",
            targets.len()
        );
    }

    for target in &targets {
        let paths = target_paths(root, target);
        std::fs::create_dir_all(&paths.corpus)
            .with_context(|| format!("create corpus dir for {target}"))?;

        eprintln!("fuzz: running {target} for {budget}s …");
        run_target(sh, &paths, target, budget)?;
        eprintln!("fuzz: {target} completed ({budget}s, no crashes)");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A manifest mixing the real cargo-fuzz shapes with bins that must be
    /// excluded: a plain helper binary and a prefix-named libtest fixture.
    const FIXTURE: &str = r#"
[package]
name = "minibox-fuzz"
version = "0.0.0"

[package.metadata]
cargo-fuzz = true

[workspace]

[[bin]]
name = "fuzz_decode_request"
path = "fuzz_targets/fuzz_decode_request.rs"
test = false
doc = false
bench = false

[[bin]]
name = "fuzz_verify_digest"
path = "fuzz_targets/fuzz_verify_digest.rs"
test = false
harness = false

[[bin]]
name = "fuzz_parse_volume"
path = "fuzz_targets/fuzz_parse_volume.rs"
test = false
harness = false
doc = false
bench = false

[[bin]]
name = "corpus-miner"
path = "src/bin/corpus_miner.rs"

[[bin]]
name = "fuzz_regression_harness"
path = "fuzz_targets/fuzz_regression_harness.rs"
test = true
harness = true
"#;

    /// Materialise `source` as the fuzz manifest under a fresh temporary root.
    fn temp_root(source: &str) -> TempDir {
        let root = TempDir::new().expect("create temp workspace root");
        let crate_dir = root.path().join(FUZZ_CRATE_DIR);
        std::fs::create_dir_all(&crate_dir).expect("create fuzz crate dir");
        std::fs::write(crate_dir.join(FUZZ_MANIFEST_FILE), source).expect("write fuzz manifest");
        root
    }

    #[test]
    fn discover_targets_enumerates_libfuzzer_bins_only() {
        let root = temp_root(FIXTURE);
        let targets = discover_targets(root.path()).expect("discover targets");

        assert_eq!(
            targets,
            vec![
                "fuzz_decode_request".to_owned(),
                "fuzz_verify_digest".to_owned(),
                "fuzz_parse_volume".to_owned(),
            ],
            "must skip the non-fuzz bin and the harness-enabled bin"
        );
    }

    #[test]
    fn discover_targets_accepts_bare_prefix_target() {
        let root = temp_root(
            r#"
[[bin]]
name = "fuzz_bare_target"
path = "fuzz_targets/fuzz_bare_target.rs"
"#,
        );
        let targets = discover_targets(root.path()).expect("discover targets");
        assert_eq!(targets, vec!["fuzz_bare_target".to_owned()]);
    }

    #[test]
    fn discover_targets_reads_manifest_relative_to_root() {
        let root = temp_root(FIXTURE);
        let expected = root.path().join(FUZZ_CRATE_DIR).join(FUZZ_MANIFEST_FILE);
        assert_eq!(fuzz_manifest_path(root.path()), expected);
        assert!(discover_targets(root.path()).is_ok());
    }

    #[test]
    fn discover_targets_reports_missing_manifest() {
        let root = TempDir::new().expect("create empty temp root");
        let error = discover_targets(root.path())
            .expect_err("missing manifest must be reported")
            .to_string();
        assert!(
            error.contains("read fuzz manifest"),
            "expected a read-failure context, got: {error}"
        );
    }

    #[test]
    fn discover_targets_reports_malformed_manifest() {
        let root = temp_root("[[bin]\nname = broken");
        let error = discover_targets(root.path())
            .expect_err("malformed manifest must be reported")
            .to_string();
        assert!(
            error.contains("parse fuzz manifest"),
            "expected a parse-failure context, got: {error}"
        );
    }

    #[test]
    fn repository_layout_exposes_every_declared_target() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .expect("xtask lives inside the workspace root");
        let manifest = fuzz_manifest_path(repo_root);
        assert!(
            manifest.is_file(),
            "expected the fuzz manifest at {}",
            manifest.display()
        );

        let targets = discover_targets(repo_root).expect("discover repository targets");
        assert!(
            targets.len() >= 19,
            "expected the full declared target set, got {}: {targets:?}",
            targets.len()
        );
        assert!(
            targets.iter().all(|t| t.starts_with(TARGET_PREFIX)),
            "every discovered target must carry the cargo-fuzz prefix: {targets:?}"
        );
        assert!(targets.contains(&"fuzz_decode_request".to_owned()));
    }

    #[test]
    fn target_paths_anchor_to_workspace_root() {
        let paths = target_paths(Path::new("/ws"), "fuzz_decode_request");
        assert_eq!(
            paths.manifest,
            Path::new("/ws/fuzz/Cargo.toml"),
            "manifest must resolve from the workspace root"
        );
        assert_eq!(
            paths.corpus,
            Path::new("/ws/fuzz/corpus/fuzz_decode_request")
        );
        assert_eq!(paths.seeds, Path::new("/ws/fuzz/seeds/fuzz_decode_request"));
    }

    #[test]
    fn target_paths_are_independent_of_process_cwd() {
        let cwd = std::env::current_dir().expect("resolve process cwd");

        // A root that is deliberately *not* the process CWD: the resolver must
        // anchor to the argument, so nothing about the CWD can leak in.
        let elsewhere = TempDir::new().expect("create unrelated root");
        let paths = target_paths(elsewhere.path(), "fuzz_decode_request");

        assert!(
            paths.manifest.starts_with(elsewhere.path()),
            "manifest must be rooted at the supplied root, got {}",
            paths.manifest.display()
        );
        assert!(paths.corpus.starts_with(elsewhere.path()));
        assert!(paths.seeds.starts_with(elsewhere.path()));
        for resolved in [&paths.manifest, &paths.corpus, &paths.seeds] {
            assert!(
                !resolved.starts_with(&cwd),
                "{} must not be relative to the process cwd {}",
                resolved.display(),
                cwd.display()
            );
            assert!(
                resolved.is_absolute(),
                "{} must be absolute when root is absolute",
                resolved.display()
            );
        }
    }

    #[test]
    fn target_paths_keep_the_same_shape_for_any_root() {
        let outer = TempDir::new().expect("create outer root");
        let inner = outer.path().join("crates/minibox");
        std::fs::create_dir_all(&inner).expect("create nested root");

        let from_outer = target_paths(outer.path(), "fuzz_verify_digest");
        let from_inner = target_paths(&inner, "fuzz_verify_digest");

        assert_eq!(
            from_outer.corpus.strip_prefix(outer.path()),
            from_inner.corpus.strip_prefix(&inner),
            "corpus layout must be relative to whichever root is supplied"
        );
        assert_eq!(
            from_outer.seeds.strip_prefix(outer.path()),
            from_inner.seeds.strip_prefix(&inner)
        );
    }

    #[test]
    fn options_default_to_the_full_suite_budget() {
        let options = Options::from_args(&[]);
        assert_eq!(options.time_secs, DEFAULT_TIME_SECS);
        assert!(!options.smoke);
        assert_eq!(options.target, None);
    }

    #[test]
    fn options_parse_target_and_time() {
        let args = vec![
            "--time".to_owned(),
            "300".to_owned(),
            "--target".to_owned(),
            "fuzz_parse_volume".to_owned(),
        ];
        let options = Options::from_args(&args);
        assert_eq!(options.time_secs, 300);
        assert_eq!(options.target.as_deref(), Some("fuzz_parse_volume"));
        assert!(!options.smoke);
    }

    #[test]
    fn options_smoke_bounds_the_budget() {
        let options = Options::from_args(&["--smoke".to_owned()]);
        assert!(options.smoke, "--smoke must be recognised");
        assert_eq!(
            options.time_secs, SMOKE_TIME_SECS,
            "smoke mode must use the bounded CI budget"
        );
        assert!(
            options.time_secs < DEFAULT_TIME_SECS,
            "smoke budget must be strictly shorter than the full run"
        );
    }

    #[test]
    fn options_explicit_time_overrides_the_smoke_default() {
        let args = vec!["--smoke".to_owned(), "--time".to_owned(), "42".to_owned()];
        assert_eq!(Options::from_args(&args).time_secs, 42);
    }

    #[test]
    fn options_ignore_unknown_and_unparsable_flags() {
        let args = vec![
            "--jobs".to_owned(),
            "1".to_owned(),
            "--time".to_owned(),
            "not-a-number".to_owned(),
        ];
        assert_eq!(Options::from_args(&args).time_secs, DEFAULT_TIME_SECS);
    }

    #[test]
    fn fuzz_args_pin_paths_and_budget() {
        let paths = target_paths(Path::new("/ws"), "fuzz_decode_request");
        let args = fuzz_args(&paths, "fuzz_decode_request", 60);

        assert_eq!(
            args,
            vec![
                "+nightly",
                "fuzz",
                "run",
                "fuzz_decode_request",
                "--manifest-path",
                "/ws/fuzz/Cargo.toml",
                "/ws/fuzz/corpus/fuzz_decode_request",
                "/ws/fuzz/seeds/fuzz_decode_request",
                "--",
                "-max_total_time=60",
            ]
        );
    }

    #[test]
    fn fuzz_args_differ_between_smoke_and_full_budgets() {
        let paths = target_paths(Path::new("/ws"), "fuzz_verify_digest");

        let full = fuzz_args(&paths, "fuzz_verify_digest", DEFAULT_TIME_SECS);
        let smoke = fuzz_args(&paths, "fuzz_verify_digest", SMOKE_TIME_SECS);

        assert_eq!(full.last().expect("budget flag"), "-max_total_time=60");
        assert_eq!(smoke.last().expect("budget flag"), "-max_total_time=5");
        assert_eq!(
            full.len(),
            smoke.len(),
            "smoke and full runs must build the same command shape"
        );
        for (full_arg, smoke_arg) in full.iter().zip(&smoke) {
            if full_arg.starts_with("-max_total_time=") {
                continue;
            }
            assert_eq!(full_arg, smoke_arg, "only the budget may differ");
        }
    }

    #[test]
    fn select_targets_defaults_to_every_discovered_target() {
        let discovered = vec!["fuzz_a".to_owned(), "fuzz_b".to_owned()];
        assert_eq!(
            select_targets(None, &discovered).expect("select"),
            discovered
        );
    }

    #[test]
    fn select_targets_honours_an_explicit_target() {
        let discovered = vec!["fuzz_a".to_owned(), "fuzz_b".to_owned()];
        assert_eq!(
            select_targets(Some("fuzz_b"), &discovered).expect("select"),
            vec!["fuzz_b".to_owned()]
        );
    }

    #[test]
    fn select_targets_rejects_unknown_target_and_lists_available() {
        let discovered = vec!["fuzz_a".to_owned(), "fuzz_b".to_owned()];
        let error = select_targets(Some("fuzz_nope"), &discovered)
            .expect_err("unknown target must be rejected")
            .to_string();
        assert!(error.contains("unknown fuzz target"), "got: {error}");
        assert!(error.contains("fuzz_a, fuzz_b"), "got: {error}");
    }
}
