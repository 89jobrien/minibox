//! `minibox-cni-verify` — operational rollout preflight for the
//! CNI-backed network provider.
//!
//! Reads the same environment `miniboxd` reads, then checks that the
//! `.conflist`, every plugin binary its chain names, and the spec version
//! each binary advertises all agree with the crate's
//! [`minibox_cni::CNI_SPEC_VERSION`].
//!
//! Prints a JSON report and exits 0 when the rollout is converged. On
//! failure it prints the error object and the install hint to stderr and
//! exits 1, so an operator can read the expected layout directly off the
//! failure.

use minibox_cni::adapters::{ExecPluginProbe, ProcessEnvironment};
use minibox_cni::config::NetworkConfigList;
use minibox_cni::plugin::error_payload;
use minibox_cni::rollout::CniRollout;
use std::process::ExitCode;

fn main() -> ExitCode {
    let rollout = CniRollout::from_environment(&ProcessEnvironment);
    let conflist_path = rollout.conflist_path();

    let conflist = match NetworkConfigList::from_file(&conflist_path) {
        Ok(conflist) => conflist,
        Err(err) => return fail(&rollout, &err),
    };
    match rollout.preflight(&conflist, &ExecPluginProbe) {
        Ok(report) => match serde_json::to_string(&report) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(err) => {
                eprintln!("minibox-cni-verify: could not serialise report: {err}");
                ExitCode::FAILURE
            }
        },
        Err(err) => fail(&rollout, &err),
    }
}

fn fail(rollout: &CniRollout, error: &minibox_cni::CniError) -> ExitCode {
    let payload = error_payload(error);
    eprintln!("minibox-cni-verify: {}", payload.msg);
    eprintln!("minibox-cni-verify: {}", rollout.install_hint());
    if let Ok(json) = serde_json::to_string(&payload) {
        println!("{json}");
    }
    ExitCode::FAILURE
}
