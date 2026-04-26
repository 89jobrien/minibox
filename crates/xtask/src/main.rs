//! xtask — workspace dev-tool binary.
//!
//! Each module has one clear responsibility. Add new tasks by creating a new
//! module and wiring it into the `match` below; do NOT grow existing modules
//! beyond their stated scope.
//!
//! | Module      | Responsibility                                              |
//! |-------------|-------------------------------------------------------------|
//! | `gates`     | Quality gates: fmt-check, clippy, nextest, coverage         |
//! | `cleanup`   | State cleanup: kill orphans, unmount overlays, rm artifacts |
//! | `vm_image`  | VM image build: Alpine kernel + minibox agent (macOS/vz)    |
//! | `vm_run`    | VM boot: interactive shell or test execution under QEMU      |

use anyhow::{Result, bail};
use std::{env, path::Path};
use xshell::Shell;

mod bump;
mod cas;
mod cgroup_tests;
mod cleanup;
mod gates;
mod preflight;
mod protocol_sites;
mod test_image;
mod test_linux;
mod vm_image;
mod vm_run;

fn main() -> Result<()> {
    let task = env::args().nth(1);

    // Set process CWD to workspace root before Shell::new() so xshell does not
    // inherit a stale/missing directory (e.g. a deleted git worktree).
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    env::set_current_dir(root)?;

    let sh = Shell::new()?;
    sh.change_dir(root);

    match task.as_deref() {
        Some("bump") => {
            let level = env::args().nth(2).unwrap_or_else(|| "patch".to_string());
            bump::bump(root, &level)
        }
        Some("preflight") => {
            preflight::require_tools(&preflight::ProcessProbe, &["cargo", "cargo-nextest", "gh"])
        }
        Some("available") => preflight::check_xtask_available(&preflight::ProcessXtaskProbe),
        Some("pre-commit") => gates::pre_commit(&sh),
        Some("prepush") => gates::prepush(&sh),
        Some("test-unit") => gates::test_unit(&sh),
        Some("test-conformance") => gates::test_conformance(&sh),
        Some("test-krun-conformance") => gates::test_krun_conformance(&sh),
        Some("test-property") => gates::test_property(&sh),
        Some("test-integration") => gates::test_integration(&sh),
        Some("test-e2e-suite") => gates::test_e2e_suite(&sh),
        Some("test-sandbox") => gates::test_sandbox(&sh),
        Some("clean-artifacts") => cleanup::clean_artifacts(&sh),
        Some("nuke-test-state") => cleanup::nuke_test_state(&sh),
        Some("build-vm-image") => {
            let force = env::args().any(|a| a == "--force");
            let vm_dir = vm_image::default_vm_dir();
            vm_image::build_vm_image(&vm_dir, force)
        }
        Some("run-vm") => {
            let vm_dir = vm_image::default_vm_dir();
            let platform = vm_run::HostPlatform::detect()?;
            vm_run::run_vm_interactive(&vm_dir, &platform)
        }
        Some("test-vm") => {
            let vm_dir = vm_image::default_vm_dir();
            let cargo_target = env::var("CARGO_TARGET_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|_| std::path::Path::new("target").to_path_buf());
            let platform = vm_run::HostPlatform::detect()?;
            vm_run::test_vm(&vm_dir, &cargo_target, &platform)
        }
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
        Some("coverage-check") => gates::coverage_check(&sh),
        Some("test-linux") => test_linux::test_linux(),
        Some("run-cgroup-tests") => cgroup_tests::run_cgroup_tests(root),
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
            eprintln!("  available        verify cargo xtask is runnable (real capability check)");
            eprintln!("  pre-commit       fmt-check + lint + build-release");
            eprintln!("  prepush          nextest + coverage");
            eprintln!("  test-unit        all unit + conformance tests");
            eprintln!("  test-conformance commit+build+push conformance suite + artifact reports");
            eprintln!(
                "  test-krun-conformance krun adapter conformance (HVF/KVM, sets MINIBOX_KRUN_TESTS=1)"
            );
            eprintln!("  test-property    property-based tests (proptest)");
            eprintln!("  test-integration cgroup + integration tests (Linux, root)");
            eprintln!("  test-e2e-suite   daemon+CLI e2e tests (Linux, root)");
            eprintln!("  test-sandbox     sandbox contract tests (Linux, root, Docker Hub)");
            eprintln!("  clean-artifacts  remove non-critical build outputs");
            eprintln!("  nuke-test-state  kill orphans, unmount overlays, clean cgroups");
            eprintln!(
                "  build-vm-image   download Alpine kernel/rootfs, cross-compile agent, build initramfs"
            );
            eprintln!(
                "  run-vm           boot VM with interactive shell (QEMU HVF, Ctrl-A X to exit)"
            );
            eprintln!("  test-vm          build musl test binaries + run in VM, stream results");
            eprintln!("  build-test-image cross-compile test binaries + assemble OCI tarball");
            eprintln!(
                "  test-linux       build image + load into minibox + run tests in container"
            );
            eprintln!(
                "  cas-add <file> [--ref <name>]  add file to CAS overlay store (~/.minibox/vm/overlay/cas/)"
            );
            eprintln!("  coverage-check   llvm-cov minibox; fail if handler.rs fns < 80%");
            eprintln!("  cas-check        verify all overlay refs match their CAS objects");
            eprintln!("  run-cgroup-tests run cgroup v2 integration tests in delegated hierarchy (Linux, root)");
            eprintln!("  check-protocol-sites [<file>] [--expected N] [--warn-only]");
            eprintln!("                   verify HandlerDependencies construction site count in miniboxd/src/main.rs");
            Ok(())
        }
    }
}
