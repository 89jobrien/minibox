//! xtask — workspace dev-tool binary.
//!
//! Commands are organized into subcommand groups:
//!
//! | Group    | Purpose                                                        |
//! |----------|----------------------------------------------------------------|
//! | `test`   | Test suites: unit, conformance, property, e2e, integration     |
//! | `check`  | Static checks: stale names, protocol drift, unwrap, coverage   |
//! | `docs`   | Documentation: audit, lint, update-date                        |
//! | `info`   | Introspection: metrics, context, detect-changes                |
//! | (top)    | Gates, CI, cleanup, promotion, and other standalone commands   |

use anyhow::{Result, bail};
use std::env;
use xshell::Shell;

mod bench;
mod borrow_fixtures;
mod bump;
mod cas;
mod cgroup_tests;
mod check_protocol_sites;
pub mod checkpoint;
mod ci_watch;
mod cleanup;
mod clippy_sarif;
mod collect_metrics;
mod context;
mod council;
mod daily_orchestration;
mod demo;
mod detect_changes;
mod docs_audit;
mod docs_lint;
mod dotenv;
mod feature_matrix_date;
mod fuzz;
mod gates;
mod lint_paths;
mod preflight;
mod promote;
mod protocol_drift;
mod protocol_sites;
mod sarif;
mod setup_test_vm;
mod stale_names;
mod test_image;
mod test_in_vm;
mod test_linux;
mod utils;
mod xconfig;

fn main() -> Result<()> {
    let argv: Vec<String> = env::args().collect();
    let task = argv.get(1).cloned();

    let sh = Shell::new()?;
    let root = sh.current_dir();
    let root = root
        .ancestors()
        .find(|p| p.join("Cargo.lock").exists())
        .unwrap_or(&root)
        .to_path_buf();
    let root = root.as_path();
    sh.change_dir(root);

    match task.as_deref() {
        // ── Subcommand groups ────────────────────────────────────────
        Some("test") => cmd_test(&sh, root),
        Some("check") => cmd_check(&sh, root, &argv[2..]),
        Some("docs") => cmd_docs(&sh, root),
        Some("info") => cmd_info(&sh, root, &argv[2..]),

        // ── Quality gates (top-level) ────────────────────────────────
        Some("verify") => gates::verify(&sh, root),
        Some("lint") => gates::lint(&sh),
        Some("fix") => gates::fix(&sh),
        Some("pre-commit") => gates::pre_commit(&sh),
        Some("prepush") => gates::prepush(&sh),
        Some("agentlint") => {
            let all = env::args().any(|a| a == "--all");
            if all {
                gates::agentlint_all()
            } else {
                gates::agentlint_staged(&sh)
            }
        }
        Some("coverage") => {
            let args: Vec<String> = env::args().collect();
            let open = args.iter().any(|a| a == "--open");
            let lcov_only = args.iter().any(|a| a == "--lcov-only");
            let html_only = args.iter().any(|a| a == "--html-only");
            gates::coverage(&sh, open, lcov_only, html_only)
        }
        Some("coverage-check") => gates::coverage_check(&sh),

        // ── Build / VM / image ───────────────────────────────────────
        Some("build-test-image") => {
            let force = env::args().any(|a| a == "--force");
            test_image::build_test_image(root, force)
        }
        Some("setup-test-vm") => {
            let force = env::args().any(|a| a == "--force");
            setup_test_vm::run(root, force)
        }
        Some("test-in-vm") => {
            let args: Vec<String> = env::args().skip(2).collect();
            let opts = test_in_vm::Options::from_args(&args);
            test_in_vm::run(root, &opts)
        }
        Some("test-linux") => {
            let cfg = xconfig::XConfig::load(root)?;
            let target_base = std::env::var("CARGO_TARGET_DIR")
                .map_or_else(|_| root.join("target"), std::path::PathBuf::from);
            let vm_dir = dirs::home_dir()
                .unwrap_or_else(|| std::path::PathBuf::from("/tmp"))
                .join(".minibox")
                .join("vm");
            let kernel = vm_dir.join("boot").join("vmlinuz-virt");

            let compiler = test_linux::ZigbuildCompiler::new(
                vec!["miniboxd".to_string(), "mbx".to_string()],
                vec!["miniboxd".to_string()],
            );
            let initramfs_builder = test_linux::CpioInitramfsBuilder;
            let vm_runner = test_linux::SmolvmRunner {
                image_name: "minibox-tester:latest".to_string(),
            };

            test_linux::run_pipeline(
                &compiler,
                &initramfs_builder,
                &vm_runner,
                &cfg.cross.target,
                &vm_dir,
                &target_base,
                &kernel,
            )
        }

        // ── Cleanup ──────────────────────────────────────────────────
        Some("clean-artifacts") => cleanup::clean_artifacts(&sh),
        Some("nuke-test-state") => cleanup::nuke_test_state(&sh),

        // ── CI / promotion / orchestration ───────────────────────────
        Some("bump") => {
            let level = env::args().nth(2).unwrap_or_else(|| "patch".to_string());
            bump::bump(root, &level)
        }
        Some("preflight") => {
            preflight::require_tools(&preflight::ProcessProbe, &["cargo", "cargo-nextest", "gh"])
        }
        Some("doctor") => preflight::doctor(&preflight::ProcessProbe),
        Some("available") => preflight::check_xtask_available(&preflight::ProcessXtaskProbe),
        Some("promote") => {
            let args: Vec<String> = env::args().skip(2).collect();
            let dry_run = args.iter().any(|a| a == "--dry-run");
            let from = args
                .windows(2)
                .find(|w| w[0] == "--from")
                .and_then(|w| promote::Tier::from_str(&w[1]));
            let to = args
                .windows(2)
                .find(|w| w[0] == "--to")
                .and_then(|w| promote::Tier::from_str(&w[1]));
            promote::run(root, from, to, dry_run)
        }
        Some("ci-watch") => {
            let branch = {
                let args: Vec<String> = env::args().collect();
                args.windows(2)
                    .find(|w| w[0] == "--branch")
                    .map(|w| w[1].clone())
            };
            ci_watch::ci_watch(&sh, branch.as_deref())
        }
        Some("daily-orchestration") => {
            let args: Vec<String> = env::args().skip(2).collect();
            let dry_run = args.iter().any(|a| a == "--dry-run");
            let ci = args.iter().any(|a| a == "--ci");
            if args.iter().any(|a| a != "--dry-run" && a != "--ci") {
                bail!("usage: cargo xtask daily-orchestration [--ci] [--dry-run]");
            }
            daily_orchestration::run(dry_run, ci)
        }
        Some("council") => {
            let args: Vec<String> = env::args().skip(2).collect();
            let base = args
                .windows(2)
                .find(|w| w[0] == "--base")
                .map_or_else(|| "main".to_string(), |w| w[1].clone());
            let mode = args
                .windows(2)
                .find(|w| w[0] == "--mode")
                .map_or_else(|| "core".to_string(), |w| w[1].clone());
            let no_synthesis = args.iter().any(|a| a == "--no-synthesis");
            let prod = args.iter().any(|a| a == "--prod");
            council::run(root, &base, &mode, no_synthesis, prod)
        }

        // ── Misc standalone ──────────────────────────────────────────
        Some("bench") => bench::bench(&sh, root),
        Some("fuzz") => fuzz::fuzz(&sh, root),
        Some("demo") => {
            let args: Vec<String> = env::args().collect();
            let adapter = args
                .windows(2)
                .find(|w| w[0] == "--adapter")
                .map_or_else(|| "smolvm".to_string(), |w| w[1].clone());
            demo::demo(&sh, root, &adapter)
        }
        Some("borrow-fixtures") => borrow_fixtures::run(root),
        Some("clippy-sarif") => {
            let sarif_path = env::args().nth(2).map_or_else(
                || std::path::PathBuf::from("clippy.sarif"),
                std::path::PathBuf::from,
            );
            clippy_sarif::run(&sarif_path)
        }
        Some("run-cgroup-tests") => cgroup_tests::run_cgroup_tests(root),
        Some("cas-add") => {
            let file_path = env::args()
                .nth(2)
                .map(std::path::PathBuf::from)
                .ok_or_else(|| {
                    anyhow::anyhow!("usage: cargo xtask cas-add <file> [--ref <name>]")
                })?;
            let ref_name = {
                let args: Vec<String> = env::args().collect();
                args.windows(2)
                    .find(|w| w[0] == "--ref")
                    .map(|w| w[1].clone())
            };
            let overlay_dir = cas::default_overlay_dir();
            cas::cas_add(&overlay_dir, &file_path, ref_name.as_deref()).map(|_| ())
        }
        Some("cas-check") => {
            let overlay_dir = cas::default_overlay_dir();
            cas::cas_check(&overlay_dir)
        }
        Some("lint-paths") => lint_paths::run(root),
        Some("run") => {
            let script_name = env::args()
                .nth(2)
                .ok_or_else(|| anyhow::anyhow!("usage: cargo xtask run <script> [args...]"))?;
            let script_path = root.join("scripts").join(format!("{script_name}.nu"));
            if !script_path.exists() {
                let mut available: Vec<String> = std::fs::read_dir(root.join("scripts"))?
                    .filter_map(std::result::Result::ok)
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        name.strip_suffix(".nu")
                            .map(std::string::ToString::to_string)
                    })
                    .collect();
                available.sort();
                bail!(
                    "script not found: {script_name}\n\nAvailable scripts:\n  {}",
                    available.join("\n  ")
                );
            }
            let extra_args: Vec<String> = env::args().skip(3).collect();
            let mut command = std::process::Command::new("nu");
            command.arg(&script_path);
            command.args(&extra_args);
            command.current_dir(root);
            let status = command
                .status()
                .map_err(|e| anyhow::anyhow!("failed to run nu {}: {e}", script_path.display()))?;
            if !status.success() {
                bail!("script {script_name} exited with {status}");
            }
            Ok(())
        }

        // ── Backward-compat aliases (deprecated) ─────────────────────
        Some(cmd) if is_test_alias(cmd) => {
            let suite = cmd.strip_prefix("test-").unwrap_or(cmd);
            eprintln!("note: `cargo xtask {cmd}` is deprecated, use `cargo xtask test {suite}`");
            dispatch_test(&sh, root, suite)
        }
        Some(cmd) if is_check_alias(cmd) => {
            let sub = check_alias_to_sub(cmd);
            eprintln!("note: `cargo xtask {cmd}` is deprecated, use `cargo xtask check {sub}`");
            dispatch_check(&sh, root, &sub, &argv[2..])
        }
        Some(cmd) if is_docs_alias(cmd) => {
            let sub = docs_alias_to_sub(cmd);
            eprintln!("note: `cargo xtask {cmd}` is deprecated, use `cargo xtask docs {sub}`");
            dispatch_docs(&sh, root, &sub)
        }
        Some(cmd) if is_info_alias(cmd) => {
            let sub = info_alias_to_sub(cmd);
            eprintln!("note: `cargo xtask {cmd}` is deprecated, use `cargo xtask info {sub}`");
            dispatch_info(&sh, root, &sub, &argv[2..])
        }

        Some(other) => bail!("unknown task: {other}"),
        None => print_help(),
    }
}

// ── test subcommand ──────────────────────────────────────────────────────────

fn cmd_test(sh: &Shell, root: &std::path::Path) -> Result<()> {
    let suite = env::args().nth(2);
    if let Some(s) = suite.as_deref() {
        dispatch_test(sh, root, s)
    } else {
        eprintln!("Usage: cargo xtask test <suite>");
        eprintln!();
        eprintln!("Suites:");
        eprintln!("  unit              unit + conformance tests (any platform)");
        eprintln!("  conformance       commit+build+push conformance suite + reports");
        eprintln!("  krun-conformance  krun adapter conformance (HVF/KVM)");
        eprintln!("  turmoil           turmoil network simulation tests");
        eprintln!("  shuttle           shuttle concurrency tests");
        eprintln!("  property          property-based tests (proptest)");
        eprintln!("  quickcheck        quickcheck property tests");
        eprintln!("  integration       cgroup + integration tests (Linux, root)");
        eprintln!("  e2e               protocol e2e tests (any platform)");
        eprintln!("  system-suite      full-stack system tests (Linux, root)");
        eprintln!("  sandbox           sandbox contract tests (Linux, root)");
        eprintln!("  gke-profile       GKE profile unit tests");
        eprintln!("  gke-adapter       GKE adapter integration tests");
        Ok(())
    }
}

fn dispatch_test(sh: &Shell, root: &std::path::Path, suite: &str) -> Result<()> {
    match suite {
        "unit" => gates::test_unit(sh),
        "conformance" => gates::test_conformance(sh),
        "krun-conformance" => gates::test_krun_conformance(sh),
        "turmoil" => gates::test_turmoil(sh),
        "shuttle" => gates::test_shuttle(sh),
        "property" => gates::test_property(sh),
        "quickcheck" => gates::test_quickcheck(sh),
        "integration" => gates::test_integration(sh),
        "e2e" => gates::test_e2e(sh),
        "system-suite" | "e2e-suite" => gates::test_system_suite(sh),
        "sandbox" => gates::test_sandbox(sh),
        "gke-profile" => gates::test_gke_profile(sh),
        "gke-adapter" => gates::test_gke_adapter(sh),
        "cgroup" => cgroup_tests::run_cgroup_tests(root),
        other => bail!("unknown test suite: {other}"),
    }
}

fn is_test_alias(cmd: &str) -> bool {
    matches!(
        cmd,
        "test-unit"
            | "test-conformance"
            | "test-krun-conformance"
            | "test-turmoil"
            | "test-shuttle"
            | "test-property"
            | "test-quickcheck"
            | "test-integration"
            | "test-e2e"
            | "test-system-suite"
            | "test-e2e-suite"
            | "test-sandbox"
            | "test-gke-profile"
            | "test-gke-adapter"
    )
}

// ── check subcommand ─────────────────────────────────────────────────────────

fn cmd_check(sh: &Shell, root: &std::path::Path, rest: &[String]) -> Result<()> {
    if let Some(s) = rest.first() {
        dispatch_check(sh, root, s, &rest[1..])
    } else {
        eprintln!("Usage: cargo xtask check <target>");
        eprintln!();
        eprintln!("Targets:");
        eprintln!("  stale-names        audit for banned old crate/binary names");
        eprintln!(
            "  protocol-drift     verify core contract hashes [--update] [--warn-only] [--hook] [--sarif <path>]"
        );
        eprintln!(
            "  protocol-sites     verify HandlerDependencies construction site count [<file>] [--expected N] [--warn-only]"
        );
        eprintln!(
            "  protocol-variants  scan for DaemonRequest/DaemonResponse variants with no handler sites"
        );
        eprintln!("  adapter-coverage   verify each adapter has integration test files");
        eprintln!("  no-unwrap          scan production code for .unwrap() [--strict]");
        eprintln!("  repo-clean         warn if generated artifacts are tracked by git");
        Ok(())
    }
}

fn dispatch_check(sh: &Shell, root: &std::path::Path, sub: &str, rest: &[String]) -> Result<()> {
    match sub {
        "stale-names" => stale_names::check_stale_names(root),
        "protocol-drift" => {
            let args = parse_protocol_drift_args(rest);
            protocol_drift::run(
                root,
                args.update,
                args.warn_only,
                args.hook,
                args.sarif.as_deref(),
            )
        }
        "protocol-sites" => {
            let args = parse_protocol_sites_args(rest);
            let file = args
                .file
                .unwrap_or_else(|| root.join("crates/miniboxd/src/main.rs"));
            protocol_sites::check_protocol_sites(&file, args.expected, args.warn_only)
        }
        "protocol-variants" => check_protocol_sites::run(root),
        "adapter-coverage" => gates::check_adapter_coverage(sh),
        "no-unwrap" => {
            let strict = rest.iter().any(|a| a == "--strict");
            gates::check_no_unwrap(sh, strict)
        }
        "repo-clean" => {
            gates::check_repo_cleanliness(sh);
            Ok(())
        }
        other => bail!("unknown check target: {other}"),
    }
}

struct ProtocolSitesArgs {
    file: Option<std::path::PathBuf>,
    expected: usize,
    warn_only: bool,
}

fn parse_protocol_sites_args(rest: &[String]) -> ProtocolSitesArgs {
    let file = rest
        .first()
        .filter(|a| !a.starts_with("--"))
        .map(std::path::PathBuf::from);
    let expected = rest
        .windows(2)
        .find(|w| w[0] == "--expected")
        .and_then(|w| w[1].parse().ok())
        .unwrap_or(4);
    let warn_only = rest.iter().any(|a| a == "--warn-only");
    ProtocolSitesArgs {
        file,
        expected,
        warn_only,
    }
}

struct ProtocolDriftArgs {
    update: bool,
    warn_only: bool,
    hook: bool,
    sarif: Option<std::path::PathBuf>,
}

fn parse_protocol_drift_args(rest: &[String]) -> ProtocolDriftArgs {
    ProtocolDriftArgs {
        update: rest.iter().any(|a| a == "--update"),
        warn_only: rest.iter().any(|a| a == "--warn-only"),
        hook: rest.iter().any(|a| a == "--hook"),
        sarif: rest
            .windows(2)
            .find(|w| w[0] == "--sarif")
            .map(|w| std::path::PathBuf::from(&w[1])),
    }
}

fn is_check_alias(cmd: &str) -> bool {
    matches!(
        cmd,
        "check-stale-names"
            | "check-protocol-drift"
            | "check-protocol-sites"
            | "check-adapter-coverage"
            | "check-no-unwrap"
            | "check-repo-clean"
    )
}

fn check_alias_to_sub(cmd: &str) -> String {
    cmd.strip_prefix("check-").unwrap_or(cmd).to_string()
}

// ── docs subcommand ──────────────────────────────────────────────────────────

fn cmd_docs(sh: &Shell, root: &std::path::Path) -> Result<()> {
    let sub = env::args().nth(2);
    if let Some(s) = sub.as_deref() {
        dispatch_docs(sh, root, s)
    } else {
        eprintln!("Usage: cargo xtask docs <action>");
        eprintln!();
        eprintln!("Actions:");
        eprintln!("  audit [--full] [--strict]   audit docs/core/ facts vs code");
        eprintln!("  lint [--sarif <path>]       validate frontmatter + status values");
        eprintln!("  update-date                 rewrite Last-updated stamp in FEATURE_MATRIX");
        Ok(())
    }
}

fn dispatch_docs(sh: &Shell, root: &std::path::Path, sub: &str) -> Result<()> {
    match sub {
        "audit" => {
            let strict = env::args().any(|a| a == "--strict");
            let full = env::args().any(|a| a == "--full");
            let mode = if full {
                docs_audit::Mode::Full
            } else {
                docs_audit::Mode::Quick { strict }
            };
            docs_audit::run(sh, root, mode)
        }
        "lint" => {
            // skip(1) = skip the binary name; search remaining args for --sarif regardless
            // of whether we arrived via `cargo xtask docs lint` or the `lint-docs` alias.
            let args: Vec<String> = env::args().skip(1).collect();
            let sarif_path = args
                .windows(2)
                .find(|w| w[0] == "--sarif")
                .map(|w| std::path::PathBuf::from(&w[1]));
            docs_lint::lint_docs(root, sarif_path.as_deref())
        }
        "update-date" => feature_matrix_date::update_feature_matrix_date(root),
        other => bail!("unknown docs action: {other}"),
    }
}

fn is_docs_alias(cmd: &str) -> bool {
    matches!(
        cmd,
        "docs-audit" | "lint-docs" | "update-feature-matrix-date"
    )
}

fn docs_alias_to_sub(cmd: &str) -> String {
    match cmd {
        "docs-audit" => "audit".to_string(),
        "lint-docs" => "lint".to_string(),
        "update-feature-matrix-date" => "update-date".to_string(),
        _ => cmd.to_string(),
    }
}

// ── info subcommand ──────────────────────────────────────────────────────────

fn cmd_info(sh: &Shell, root: &std::path::Path, rest: &[String]) -> Result<()> {
    if let Some(s) = rest.first() {
        dispatch_info(sh, root, s, &rest[1..])
    } else {
        eprintln!("Usage: cargo xtask info <target>");
        eprintln!();
        eprintln!("Targets:");
        eprintln!("  metrics [--save]             aggregate crate count, test count, source lines");
        eprintln!("  context [--save]             machine-readable repo context snapshot");
        eprintln!("  changes [<base-ref>]         classify changed paths; emit GHA outputs");
        Ok(())
    }
}

fn dispatch_info(sh: &Shell, root: &std::path::Path, sub: &str, rest: &[String]) -> Result<()> {
    match sub {
        "metrics" => {
            let save = rest.iter().any(|a| a == "--save");
            collect_metrics::collect_metrics(root, save)
        }
        "context" => {
            let save = rest.iter().any(|a| a == "--save");
            context::context(sh, root, save)
        }
        "changes" => {
            let base_ref = changes_base_ref(rest);
            detect_changes::run(root, &base_ref)
        }
        other => bail!("unknown info target: {other}"),
    }
}

fn changes_base_ref(rest: &[String]) -> String {
    rest.first()
        .filter(|a| !a.starts_with("--"))
        .cloned()
        .unwrap_or_else(|| "HEAD^".to_string())
}

fn is_info_alias(cmd: &str) -> bool {
    matches!(cmd, "collect-metrics" | "context" | "detect-changes")
}

fn info_alias_to_sub(cmd: &str) -> String {
    match cmd {
        "collect-metrics" => "metrics".to_string(),
        "detect-changes" => "changes".to_string(),
        // "context" maps to itself
        _ => cmd.to_string(),
    }
}

// ── Help ─────────────────────────────────────────────────────────────────────

fn print_help() -> Result<()> {
    eprintln!("Usage: cargo xtask <command> [args...]");
    eprintln!();
    eprintln!("Subcommand groups:");
    eprintln!("  test <suite>       run a test suite (unit, conformance, e2e, ...)");
    eprintln!("  check <target>     static checks (stale-names, protocol-drift, no-unwrap, ...)");
    eprintln!("  docs <action>      documentation tools (audit, lint, update-date)");
    eprintln!("  info <target>      introspection (metrics, context, changes)");
    eprintln!();
    eprintln!("Quality gates:");
    eprintln!("  verify             read-only gate: fmt, clippy, check, borrow fixtures, docs");
    eprintln!("  lint               fmt-check + clippy + cargo check");
    eprintln!("  fix                fmt + clippy --fix + re-stage");
    eprintln!("  pre-commit         validation-only pre-commit checks");
    eprintln!("  prepush            release build + lib tests + conformance");
    eprintln!("  agentlint [--all]  lint agent config files");
    eprintln!("  coverage [--open] [--lcov-only] [--html-only]");
    eprintln!("  coverage-check     handler module function coverage gate");
    eprintln!();
    eprintln!("Build / VM:");
    eprintln!("  build-test-image [--force]     cross-compile + OCI tarball");
    eprintln!("  setup-test-vm [--force]        persistent smolvm VM with Rust");
    eprintln!("  test-in-vm [--skip-build] [--keep] [--smolfile <path>]");
    eprintln!("  test-linux                     build + load + run tests in container");
    eprintln!();
    eprintln!("CI / promotion:");
    eprintln!("  bump [patch|minor|major]       bump workspace version");
    eprintln!("  preflight                      check required tools");
    eprintln!("  doctor                         full preflight diagnostics");
    eprintln!("  promote [--from <tier>] [--to <tier>] [--dry-run]");
    eprintln!("  ci-watch [--branch <name>]     watch latest GHA run");
    eprintln!("  daily-orchestration [--ci] [--dry-run]");
    eprintln!("  council [--base <ref>] [--mode core|extended] [--prod]");
    eprintln!();
    eprintln!("Misc:");
    eprintln!("  bench              criterion benchmarks");
    eprintln!("  fuzz               libFuzzer protocol targets");
    eprintln!("  demo [--adapter <name>]");
    eprintln!("  borrow-fixtures    borrow-reasoning must-pass/must-fail fixtures");
    eprintln!("  clippy-sarif [<path>]");
    eprintln!("  run-cgroup-tests   cgroup v2 integration tests (Linux, root)");
    eprintln!("  clean-artifacts    remove non-critical build outputs");
    eprintln!("  nuke-test-state    kill orphans, unmount overlays, clean cgroups");
    eprintln!("  cas-add <file> [--ref <name>]");
    eprintln!("  cas-check          verify overlay refs match CAS objects");
    eprintln!("  run <script>       run scripts/<script>.nu");
    Ok(())
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod dispatch_args_tests {
    use super::*;

    #[test]
    fn protocol_sites_args_from_alias_form() {
        // argv after slicing: alias `xtask check-protocol-sites <file> --expected 4`
        // must yield rest = ["crates/miniboxd/src/main.rs", "--expected", "4"]
        let rest = vec![
            "crates/miniboxd/src/main.rs".to_string(),
            "--expected".to_string(),
            "4".to_string(),
        ];
        let parsed = parse_protocol_sites_args(&rest);
        assert_eq!(
            parsed.file.as_deref(),
            Some(std::path::Path::new("crates/miniboxd/src/main.rs"))
        );
        assert_eq!(parsed.expected, 4);
        assert!(!parsed.warn_only);
    }

    #[test]
    fn protocol_sites_args_default_file_when_flag_first() {
        let rest = vec!["--expected".to_string(), "4".to_string()];
        let parsed = parse_protocol_sites_args(&rest);
        assert!(
            parsed.file.is_none(),
            "flag must not be mistaken for the file path"
        );
        assert_eq!(parsed.expected, 4);
    }

    #[test]
    fn protocol_drift_args_sarif_first_flag() {
        // alias `xtask check-protocol-drift --sarif protocol-drift.sarif`
        let rest = vec!["--sarif".to_string(), "protocol-drift.sarif".to_string()];
        let parsed = parse_protocol_drift_args(&rest);
        assert_eq!(
            parsed.sarif.as_deref(),
            Some(std::path::Path::new("protocol-drift.sarif"))
        );
        assert!(!parsed.update && !parsed.warn_only && !parsed.hook);
    }

    #[test]
    fn protocol_drift_args_hook_warn_only() {
        let rest = vec!["--hook".to_string(), "--warn-only".to_string()];
        let parsed = parse_protocol_drift_args(&rest);
        assert!(parsed.hook);
        assert!(parsed.warn_only);
        assert!(parsed.sarif.is_none());
    }

    #[test]
    fn info_changes_base_ref_from_alias_form() {
        // alias `xtask detect-changes origin/main`
        let rest = vec!["origin/main".to_string()];
        assert_eq!(changes_base_ref(&rest), "origin/main");
        assert_eq!(changes_base_ref(&[]), "HEAD^");
    }

    #[test]
    fn check_alias_maps_protocol_sites_to_site_count_guard() {
        // The alias must map to the `check protocol-sites` sub, and no top-level
        // command may shadow it.
        assert!(is_check_alias("check-protocol-sites"));
        assert_eq!(check_alias_to_sub("check-protocol-sites"), "protocol-sites");
    }
}
