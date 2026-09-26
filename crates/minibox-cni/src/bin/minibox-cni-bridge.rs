//! `minibox-bridge` — the packaged CNI plugin binary.
//!
//! Speaks the CNI exec protocol for the bridge network mode that
//! `miniboxd` selects behind its `cni` feature: it decodes `CNI_COMMAND`,
//! validates the requested spec version against
//! [`minibox_cni::CNI_SPEC_VERSION`] — the crate's single source of truth —
//! and dispatches to a [`NamespaceAttachment`] backend.
//!
//! On success the reply is written to stdout and the process exits 0. On
//! failure a spec-shaped error object is written to stdout and the process
//! exits 1, which is what a CNI runtime reads.
//!
//! [`NamespaceAttachment`]: minibox_cni::NamespaceAttachment

use minibox_cni::CniError;
use minibox_cni::PluginEnvironment as _;
use minibox_cni::adapters::{ProcessEnvironment, UnimplementedNamespaceAttachment};
use minibox_cni::plugin::{PluginRequest, dispatch, error_payload};
use minibox_cni::rollout::BRIDGE_PLUGIN_NAME;
use minibox_cni::version::{CNI_COMMAND_VERSION, ENV_CNI_COMMAND};
use std::io::Read as _;
use std::process::ExitCode;

/// Read the plugin config from stdin.
///
/// `CNI_COMMAND=VERSION` takes no config, and a runtime may hand the plugin
/// an open stdin it never closes, so that command must not block here.
fn read_config(command: Option<&str>) -> std::io::Result<Vec<u8>> {
    if command == Some(CNI_COMMAND_VERSION) {
        return Ok(Vec::new());
    }
    let mut buffer = Vec::new();
    std::io::stdin().read_to_end(&mut buffer)?;
    Ok(buffer)
}

/// Write a value as one line of JSON on stdout, then return `success`.
fn emit(value: &serde_json::Value) -> ExitCode {
    match serde_json::to_string(value) {
        Ok(json) => {
            println!("{json}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("minibox-bridge: could not serialise reply: {err}");
            ExitCode::FAILURE
        }
    }
}

/// Write a spec-shaped error object on stdout, then return `failure`.
///
/// The spec has the runtime read the error object from stdout, so the
/// payload is printed even though the process exits non-zero.
fn emit_error(error: &CniError) -> ExitCode {
    let payload = error_payload(error);
    eprintln!("minibox-bridge: {}", payload.msg);
    match serde_json::to_string(&payload) {
        Ok(json) => println!("{json}"),
        Err(err) => eprintln!("minibox-bridge: could not serialise error: {err}"),
    }
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let env = ProcessEnvironment;
    let config = match read_config(env.get(ENV_CNI_COMMAND).as_deref()) {
        Ok(config) => config,
        Err(err) => return emit_error(&CniError::Io(err)),
    };

    // `CNI_COMMAND=VERSION` is answered entirely from the crate's constants
    // and never reaches a backend; the dispatch below handles all four
    // commands uniformly.
    let backend = UnimplementedNamespaceAttachment::new(BRIDGE_PLUGIN_NAME);
    match PluginRequest::decode(&env, &config) {
        Ok(request) => match dispatch(&request, &backend) {
            Ok(value) => emit(&value),
            Err(err) => emit_error(&err),
        },
        Err(err) => emit_error(&err),
    }
}
