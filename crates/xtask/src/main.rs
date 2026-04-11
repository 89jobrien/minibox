//! xtask — workspace dev-tool binary.
//!
//! Each module has one clear responsibility. Add new tasks by creating a new
//! module and wiring it into the `match` below; do NOT grow existing modules
//! beyond their stated scope.
//!
//! | Module      | Responsibility                                              |
//! |-------------|-------------------------------------------------------------|
//! | `gates`     | Quality gates: fmt-check, clippy, nextest, coverage         |
//! | `bench`     | Benchmark orchestration: local run, VPS run, diff, report  |
//! | `flamegraph`| Profiling: samply (macOS) / cargo-flamegraph (Linux)        |
//! | `cleanup`   | State cleanup: kill orphans, unmount overlays, rm artifacts |
//! | `vm_image`  | VM image build: Alpine kernel + minibox agent (macOS/vz)    |

use anyhow::{Result, bail};
use std::{env, path::Path};
use xshell::Shell;

mod bench;
mod bench_types;
mod bump;
mod cleanup;
mod flamegraph;
mod gates;
mod test_image;
mod vm_image;

fn main() -> Result<()> {
    let task = env::args().nth(1);
    let sh = Shell::new()?;

    // Run from workspace root
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    sh.change_dir(root);

    match task.as_deref() {
        Some("bump") => {
            let level = env::args().nth(2).unwrap_or_else(|| "patch".to_string());
            bump::bump(root, &level)
        }
        Some("pre-commit") => gates::pre_commit(&sh),
        Some("prepush") => gates::prepush(&sh),
        Some("test-unit") => gates::test_unit(&sh),
        Some("test-conformance") => gates::test_conformance(&sh),
        Some("test-property") => gates::test_property(&sh),
        Some("test-integration") => gates::test_integration(&sh),
        Some("test-e2e-suite") => gates::test_e2e_suite(&sh),
        Some("test-sandbox") => gates::test_sandbox(&sh),
        Some("clean-artifacts") => cleanup::clean_artifacts(&sh),
        Some("nuke-test-state") => cleanup::nuke_test_state(&sh),
        Some("bench") => {
            let extra: Vec<String> = env::args().skip(2).collect();
            bench::bench(&sh, &extra)
        }
        Some("bench-vps") => {
            let extra: Vec<String> = env::args().skip(2).collect();
            bench::bench_vps(&sh, &extra)
        }
        Some("bench-diff") => {
            let extra: Vec<String> = env::args().skip(2).collect();
            bench::bench_diff(&extra)
        }
        Some("bench-report") => bench::bench_report(),
        Some("bench-sync") => bench::bench_sync(),
        Some("flamegraph") => {
            let extra: Vec<String> = env::args().skip(2).collect();
            flamegraph::flamegraph(&sh, &extra)
        }
        Some("build-vm-image") => {
            let force = env::args().any(|a| a == "--force");
            let vm_dir = vm_image::default_vm_dir();
            vm_image::build_vm_image(&vm_dir, force)
        }
        Some("build-test-image") => {
            let force = env::args().any(|a| a == "--force");
            test_image::build_test_image(force)
        }
        Some("test-linux") => test_image::test_linux(&sh),
        Some(other) => bail!("unknown task: {other}"),
        None => {
            eprintln!("Available tasks:");
            eprintln!("  bump [patch|minor|major]  bump workspace version in Cargo.toml");
            eprintln!("  pre-commit       fmt-check + lint + build-release");
            eprintln!("  prepush          nextest + coverage");
            eprintln!("  test-unit        all unit + conformance tests");
            eprintln!("  test-conformance commit+build+push conformance suite + artifact reports");
            eprintln!("  test-property    property-based tests (proptest)");
            eprintln!("  test-integration cgroup + integration tests (Linux, root)");
            eprintln!("  test-e2e-suite   daemon+CLI e2e tests (Linux, root)");
            eprintln!("  test-sandbox     sandbox contract tests (Linux, root, Docker Hub)");
            eprintln!("  clean-artifacts  remove non-critical build outputs");
            eprintln!("  nuke-test-state  kill orphans, unmount overlays, clean cgroups");
            eprintln!("  bench            run benchmark binary (local, dry-run safe)");
            eprintln!(
                "  bench-vps        run benchmark on VPS, append to bench/results/bench.jsonl"
            );
            eprintln!("  bench-diff       diff two bench JSON files (default: HEAD vs previous)");
            eprintln!(
                "  bench-report     generate HTML report from bench/results/bench.jsonl
  bench-sync       rsync VPS bench.jsonl and merge new entries locally"
            );
            eprintln!(
                "  flamegraph       profile bench binary with samply (macOS) or cargo-flamegraph (Linux)"
            );
            eprintln!("  build-vm-image   download Alpine kernel/rootfs, cross-compile agent");
            eprintln!("  build-test-image cross-compile test binaries + assemble OCI tarball");
            eprintln!(
                "  test-linux       build image + load into minibox + run tests in container"
            );
            Ok(())
        }
    }
}
