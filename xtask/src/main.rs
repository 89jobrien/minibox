//! xtask — workspace dev-tool binary.
//!
//! Each module has one clear responsibility. Add new tasks by creating a new
//! module and wiring it into the `match` below; do NOT grow existing modules
//! beyond their stated scope.
//!
//! | Module               | Responsibility                                              |
//! |----------------------|-------------------------------------------------------------|
//! | `gates`              | Quality gates: fmt-check, clippy, nextest, coverage         |
//! | `cleanup`            | State cleanup: kill orphans, unmount overlays, rm artifacts |
//! | `feature_matrix_date`| Rewrite Last-updated stamp in FEATURE_MATRIX.mbx.md        |
use anyhow::{Result, bail};
use std::env;
use xshell::Shell;

mod bench;
mod borrow_fixtures;
mod bump;
mod cas;
mod cgroup_tests;
mod cleanup;
mod context;
mod daily_orchestration;
mod docs_lint;
mod feature_matrix_date;
mod gates;
mod preflight;
mod protocol_drift;
mod protocol_sites;
mod stale_names;
mod test_image;
mod test_linux;
mod utils;

fn main() -> Result<()> {
    let task = env::args().nth(1);

    let sh = Shell::new()?;
    // Resolve workspace root from the process CWD (set by git/cargo at invocation
    // time) rather than the compile-time CARGO_MANIFEST_DIR, which breaks when
    // the binary was built from a git worktree that has since been removed.
    let root = sh.current_dir();
    let root = root
        .ancestors()
        .find(|p| p.join("Cargo.lock").exists())
        .unwrap_or(&root)
        .to_path_buf();
    let root = root.as_path();
    sh.change_dir(root);

    match task.as_deref() {
        Some("bump") => {
            let level = env::args().nth(2).unwrap_or_else(|| "patch".to_string());
            bump::bump(root, &level)
        }
        Some("preflight") => {
            preflight::require_tools(&preflight::ProcessProbe, &["cargo", "cargo-nextest", "gh"])
        }
        Some("doctor") => preflight::doctor(&preflight::ProcessProbe),
        Some("available") => preflight::check_xtask_available(&preflight::ProcessXtaskProbe),
        Some("borrow-fixtures") => borrow_fixtures::run(root),
        Some("borrow") => match env::args().nth(2).as_deref() {
            Some("fixtures") => borrow_fixtures::run(root),
            Some(other) => bail!("unknown borrow task: {other}. Available: fixtures"),
            None => bail!("usage: cargo xtask borrow fixtures"),
        },
        Some("lint") => gates::lint(&sh),
        Some("fix") => gates::fix(&sh),
        Some("pre-commit") => gates::pre_commit(&sh),
        Some("prepush") => gates::prepush(&sh),
        Some("test-unit") => gates::test_unit(&sh),
        Some("test-conformance") => gates::test_conformance(&sh),
        Some("test-krun-conformance") => gates::test_krun_conformance(&sh),
        Some("test-property") => gates::test_property(&sh),
        Some("test-integration") => gates::test_integration(&sh),
        Some("test-e2e") => gates::test_e2e(&sh),
        Some("test-system-suite") => gates::test_system_suite(&sh),
        Some("test-e2e-suite") => gates::test_e2e_suite(&sh),
        Some("test-sandbox") => gates::test_sandbox(&sh),
        Some("clean-artifacts") => cleanup::clean_artifacts(&sh),
        Some("nuke-test-state") => cleanup::nuke_test_state(&sh),
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
        Some("build-test-image") => {
            let force = env::args().any(|a| a == "--force");
            test_image::build_test_image(force)
        }
        Some("check-repo-clean") => {
            gates::check_repo_cleanliness(&sh);
            Ok(())
        }
        Some("coverage-check") => gates::coverage_check(&sh),
        Some("check-adapter-coverage") => gates::check_adapter_coverage(&sh),
        Some("check-no-unwrap") => {
            let strict = env::args().any(|a| a == "--strict");
            gates::check_no_unwrap(&sh, strict)
        }
        Some("test-linux") => {
            let target_base = std::env::var("CARGO_TARGET_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| root.join("target"));
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
                "aarch64-unknown-linux-musl",
                &vm_dir,
                &target_base,
                &kernel,
            )
        }
        Some("run-cgroup-tests") => cgroup_tests::run_cgroup_tests(root),
        Some("lint-docs") => docs_lint::lint_docs(root),
        Some("bench") => bench::bench(&sh, root),
        Some("context") => {
            let save = env::args().any(|a| a == "--save");
            context::context(&sh, root, save)
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
        Some("update-feature-matrix-date") => feature_matrix_date::update_feature_matrix_date(root),
        Some("check-stale-names") => stale_names::check_stale_names(root),
        Some("check-protocol-drift") => {
            let args: Vec<String> = env::args().skip(2).collect();
            let update = args.iter().any(|a| a == "--update");
            let warn_only = args.iter().any(|a| a == "--warn-only");
            let hook = args.iter().any(|a| a == "--hook");
            if args
                .iter()
                .any(|a| a != "--update" && a != "--warn-only" && a != "--hook")
            {
                bail!("usage: cargo xtask check-protocol-drift [--update] [--warn-only] [--hook]");
            }
            protocol_drift::run(root, update, warn_only, hook)
        }
        Some("check-protocol-sites") => {
            let file = env::args()
                .nth(2)
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| root.join("crates/miniboxd/src/main.rs"));
            let args_vec: Vec<String> = env::args().collect();
            let expected: usize = args_vec
                .windows(2)
                .find(|w| w[0] == "--expected")
                .and_then(|w| w[1].parse().ok())
                .unwrap_or(3);
            let warn_only = env::args().any(|a| a == "--warn-only");
            protocol_sites::check_protocol_sites(&file, expected, warn_only)
        }
        Some(other) => bail!("unknown task: {other}"),
        None => {
            eprintln!("Available tasks:");
            eprintln!("  bump [patch|minor|major]  bump workspace version in Cargo.toml");
            eprintln!("  preflight        check required tools are on PATH and functional");
            eprintln!(
                "  doctor           full preflight: tools + CARGO_TARGET_DIR + Linux system checks"
            );
            eprintln!("  available        verify cargo xtask is runnable (real capability check)");
            eprintln!("  lint             fmt-check + clippy + cargo check (CI lint gate)");
            eprintln!("  fix              fmt + clippy --fix + re-stage (mutates files)");
            eprintln!("  pre-commit       validation-only: fmt-check + clippy (no file mutations)");
            eprintln!("  prepush          fast lib tests (debug, incremental)");
            eprintln!("  test-unit        all unit + conformance tests");
            eprintln!("  test-conformance commit+build+push conformance suite + artifact reports");
            eprintln!(
                "  test-krun-conformance krun adapter conformance (HVF/KVM, sets MINIBOX_KRUN_TESTS=1)"
            );
            eprintln!("  test-property    property-based tests (proptest)");
            eprintln!("  test-integration cgroup + integration tests (Linux, root)");
            eprintln!("  test-e2e         protocol e2e tests (any platform, no root required)");
            eprintln!("  test-system-suite full-stack system tests (Linux, root, cgroups v2)");
            eprintln!("  test-e2e-suite   alias for test-system-suite (backward compat)");
            eprintln!("  test-sandbox     sandbox contract tests (Linux, root, Docker Hub)");
            eprintln!("  clean-artifacts  remove non-critical build outputs");
            eprintln!("  nuke-test-state  kill orphans, unmount overlays, clean cgroups");
            eprintln!("  build-test-image cross-compile test binaries + assemble OCI tarball");
            eprintln!(
                "  test-linux       build image + load into minibox + run tests in container"
            );
            eprintln!(
                "  cas-add <file> [--ref <name>]  add file to CAS overlay store (~/.minibox/vm/overlay/cas/)"
            );
            eprintln!(
                "  check-repo-clean warn if generated artifacts (target/, traces/, *.profraw) are tracked"
            );
            eprintln!("  coverage-check   llvm-cov minibox; fail if handler.rs fns < 80%");
            eprintln!(
                "  check-adapter-coverage  verify each wired adapter has integration test files"
            );
            eprintln!(
                "  check-no-unwrap [--strict]  scan production code for .unwrap() (advisory by default)"
            );
            eprintln!(
                "  lint-docs        validate frontmatter + status values in docs/superpowers/"
            );
            eprintln!("  bench            run criterion benchmarks, save to bench/results/");
            eprintln!("  cas-check        verify all overlay refs match their CAS objects");
            eprintln!(
                "  run-cgroup-tests run cgroup v2 integration tests in delegated hierarchy (Linux, root)"
            );
            eprintln!(
                "  update-feature-matrix-date  rewrite Last-updated stamp in docs/FEATURE_MATRIX.mbx.md to today"
            );
            eprintln!("  check-stale-names audit workspace for banned old crate/binary names");
            eprintln!(
                "  check-protocol-drift [--update] [--warn-only] [--hook]  verify core contract hashes"
            );
            eprintln!("  context [--save]  dump machine-readable repo context snapshot (JSON)");
            eprintln!(
                "  daily-orchestration [--ci] [--dry-run]  run the Claude daily orchestration workflow"
            );
            eprintln!("  check-protocol-sites [<file>] [--expected N] [--warn-only]");
            eprintln!(
                "                   verify HandlerDependencies construction site count in miniboxd/src/main.rs"
            );
            Ok(())
        }
    }
}
