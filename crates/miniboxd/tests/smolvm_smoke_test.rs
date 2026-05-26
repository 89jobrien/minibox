mod helpers;

use helpers::smolvm::{SmolVmFixture, smolvm_available};
use std::path::Path;

#[test]
#[ignore = "requires smolvm on PATH"]
fn smolvm_fixture_boots_minimal_and_runs_uname() {
    if !smolvm_available() {
        eprintln!("skipping: smolvm not on PATH");
        return;
    }

    let vm = SmolVmFixture::start("smoke", Path::new("tests/smolfiles/minimal.smolfile"));
    let result = vm.exec(&["uname", "-s"]);
    assert!(result.success, "uname failed: {}", result.stderr);
    assert_eq!(result.stdout.trim(), "Linux", "VM should be running Linux");
}

#[test]
#[ignore = "requires smolvm on PATH"]
fn smolvm_fixture_boots_ci_gate_and_checks_workspace() {
    if !smolvm_available() {
        eprintln!("skipping: smolvm not on PATH");
        return;
    }

    let vm = SmolVmFixture::start("ci-smoke", Path::new("tests/smolfiles/ci-gate.smolfile"));
    let result = vm.exec(&["test", "-f", "/workspace/Cargo.toml"]);
    assert!(result.success, "workspace not mounted: {}", result.stderr);
}
