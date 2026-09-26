//! minibox-crux-plugin library — crux JSON-RPC plugin logic for minibox container operations.
//!
//! Exposes the minibox container API as crux handlers under the
//! `minibox::container::*` and `minibox::image::*` namespaces.
//!
//! # Input contract
//!
//! Each handler's declared inputs ([`handler_decls`]) are the handler's
//! *complete* input contract, and they are enforced by [`build_request`]:
//!
//! - every declared field is bound into the outgoing [`DaemonRequest`];
//! - any field a caller supplies that is not declared is **rejected** with an
//!   error naming the handler and the offending field.
//!
//! The second rule is deliberate. The daemon protocol carries more fields than
//! this plugin binds (for example [`DaemonRequest::Run`] has `ephemeral`,
//! `network`, `tty`, and `entrypoint`). Accepting those keys and dropping them
//! would let a Crux planner believe a capability was applied when it was not, so
//! the plugin refuses them loudly instead of ignoring them.
//!
//! [`DaemonRequest`]: minibox_core::protocol::DaemonRequest
//! [`DaemonRequest::Run`]: minibox_core::protocol::DaemonRequest::Run
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::doc_markdown,
        clippy::unwrap_in_result
    )
)]

use crate::protocol::{HandlerDecl, HandlerInput, Request, Response};
use anyhow::{Context, Result};
use minibox_core::client::{DaemonClient, default_socket_path};
use minibox_core::domain::BindMount;
use minibox_core::protocol::DaemonRequest;
use serde_json::Value;
use tracing::{debug, warn};

pub mod protocol;

// ── Handler declarations ───────────────────────────────────────────────────────

/// One input field in a handler's contract.
#[derive(Debug, Clone, Copy)]
struct InputSpec {
    /// Field name as it appears in the handler's `input` object.
    name: &'static str,
    /// Whether the handler fails without it.
    required: bool,
}

/// Declares a required input field.
const fn required(name: &'static str) -> InputSpec {
    InputSpec {
        name,
        required: true,
    }
}

/// Declares an optional input field.
const fn optional(name: &'static str) -> InputSpec {
    InputSpec {
        name,
        required: false,
    }
}

/// The declaration for a single handler: its identity, prose, and input contract.
struct HandlerSpec {
    /// Namespaced handler name.
    name: &'static str,
    /// One-line summary used as the description prefix.
    summary: &'static str,
    /// Complete set of accepted input fields.
    inputs: &'static [InputSpec],
    /// Extra prose appended after the rendered input list.
    notes: &'static str,
}

/// Every handler this plugin exposes, in declaration order.
///
/// This table is the single source of truth: [`handler_decls`] renders the
/// `Declare` payload from it, and [`build_request`] validates incoming input
/// against it, so the advertised surface and the enforced surface cannot drift.
const HANDLERS: &[HandlerSpec] = &[
    HandlerSpec {
        name: "minibox::container::run",
        summary: "Create and start a container (non-ephemeral)",
        inputs: &[
            required("image"),
            optional("tag"),
            optional("command"),
            optional("env"),
            optional("mounts"),
            optional("memory_limit_bytes"),
            optional("cpu_weight"),
            optional("name"),
            optional("platform"),
            optional("privileged"),
        ],
        notes: "mounts entries are {host_path, container_path, read_only} with absolute paths; \
                env entries are KEY=VALUE strings.",
    },
    HandlerSpec {
        name: "minibox::container::stop",
        summary: "Stop a running container",
        inputs: &[required("id")],
        notes: "",
    },
    HandlerSpec {
        name: "minibox::container::pause",
        summary: "Pause (freeze) a running container via cgroup.freeze",
        inputs: &[required("id")],
        notes: "",
    },
    HandlerSpec {
        name: "minibox::container::resume",
        summary: "Resume (thaw) a paused container",
        inputs: &[required("id")],
        notes: "",
    },
    HandlerSpec {
        name: "minibox::container::rm",
        summary: "Remove a stopped container",
        inputs: &[required("id")],
        notes: "",
    },
    HandlerSpec {
        name: "minibox::container::exec",
        summary: "Execute a command in a running container (Linux native only)",
        inputs: &[
            required("id"),
            required("command"),
            optional("env"),
            optional("tty"),
        ],
        notes: "env entries are KEY=VALUE strings.",
    },
    HandlerSpec {
        name: "minibox::container::ps",
        summary: "List all containers",
        inputs: &[],
        notes: "",
    },
    HandlerSpec {
        name: "minibox::container::logs",
        summary: "Fetch logs for a container",
        inputs: &[required("id")],
        notes: "",
    },
    HandlerSpec {
        name: "minibox::image::pull",
        summary: "Pull an image from a registry",
        inputs: &[required("image"), optional("tag"), optional("platform")],
        notes: "",
    },
    HandlerSpec {
        name: "minibox::image::build",
        summary: "Build an image from a Dockerfile",
        inputs: &[
            required("context_path"),
            optional("tag"),
            optional("dockerfile"),
        ],
        notes: "tag defaults to \"latest\"; dockerfile defaults to \"FROM scratch\".",
    },
    HandlerSpec {
        name: "minibox::image::push",
        summary: "Push an image to a registry",
        inputs: &[required("image")],
        // The destination is derived from the image reference's own registry and
        // repository. `DaemonRequest::Push` carries no separate destination and
        // the `ImagePusher` port resolves local cache key and remote destination
        // from the same `ImageRef`, so a second destination cannot be expressed
        // without changing the protocol and the port. Rather than advertise a
        // field that cannot be honored, the input contract omits it and rejects
        // it on arrival.
        notes: "The image reference carries its own registry and repository; anonymous \
                credentials are used.",
    },
    HandlerSpec {
        name: "minibox::image::ls",
        summary: "List all cached images",
        inputs: &[],
        notes: "",
    },
    HandlerSpec {
        name: "minibox::image::rm",
        summary: "Remove a cached image by reference",
        inputs: &[required("image_ref")],
        notes: "",
    },
];

/// Looks up a handler's declaration spec, or `None` for an unknown name.
#[must_use]
fn handler_spec(name: &str) -> Option<&'static HandlerSpec> {
    HANDLERS.iter().find(|spec| spec.name == name)
}

/// Renders a spec's `description` from its summary, input list, and notes.
///
/// The `Input: {…}` portion is derived from [`HandlerSpec::inputs`] rather than
/// written by hand, so the prose a planner reads cannot claim a field the
/// enforced contract does not accept.
fn render_description(spec: &HandlerSpec) -> String {
    let inputs = if spec.inputs.is_empty() {
        "{}".to_string()
    } else {
        let names: Vec<&str> = spec.inputs.iter().map(|i| i.name).collect();
        format!("{{{}}}", names.join(", "))
    };
    if spec.notes.is_empty() {
        format!("{}. Input: {inputs}", spec.summary)
    } else {
        format!("{}. Input: {inputs} {}", spec.summary, spec.notes)
    }
}

/// All handlers exposed by this plugin, in declaration order.
#[must_use]
pub fn handler_decls() -> Vec<HandlerDecl> {
    HANDLERS
        .iter()
        .map(|spec| HandlerDecl {
            name: spec.name.to_string(),
            description: render_description(spec),
            inputs: spec
                .inputs
                .iter()
                .map(|input| HandlerInput {
                    name: input.name.to_string(),
                    required: input.required,
                })
                .collect(),
        })
        .collect()
}

// ── Handler dispatch ───────────────────────────────────────────────────────────

/// Route a handler invocation to the appropriate daemon request.
///
/// Returns `Ok(Value)` on success (the JSON response payload) or `Err` on
/// failure.  The caller is responsible for wrapping the result in
/// `Response::InvokeOk` / `Response::InvokeErr`.
pub async fn dispatch(handler: &str, input: Value) -> Result<Value> {
    let client = DaemonClient::with_socket(default_socket_path());

    let request = build_request(handler, &input)
        .with_context(|| format!("build_request for handler '{handler}'"))?;

    debug!(handler, "dispatching to daemon");

    let mut stream = client
        .call(request)
        .await
        .with_context(|| format!("daemon call for handler '{handler}'"))?;

    // Collect all responses until a terminal response arrives or the stream
    // closes. `ContainerCreated` is non-terminal per the protocol contract:
    // for non-ephemeral runs the daemon sends it and then drops its sender,
    // so `stream.next()` returns `None` and the loop exits on stream close.
    let mut responses: Vec<Value> = Vec::new();
    while let Some(resp) = stream.next().await.context("read daemon response")? {
        let is_terminal = resp.is_terminal();
        let json = serde_json::to_value(&resp).context("serialize DaemonResponse")?;
        responses.push(json);
        if is_terminal {
            break;
        }
    }

    // Return the terminal response (last element) or the full array for
    // streaming handlers.
    match responses.len() {
        0 => anyhow::bail!("daemon returned no response"),
        1 => Ok(responses.remove(0)),
        _ => Ok(Value::Array(responses)),
    }
}

/// Map a handler name + JSON input to the appropriate `DaemonRequest`.
///
/// The handler must be one of [`handler_decls`], and `input` must be a JSON
/// object whose fields are all declared for that handler — see the crate-level
/// "Input contract" section. Undeclared fields are rejected rather than
/// dropped, so an unimplemented capability is reported to the caller instead
/// of being silently discarded.
pub fn build_request(handler: &str, input: &Value) -> Result<DaemonRequest> {
    let spec =
        handler_spec(handler).ok_or_else(|| anyhow::anyhow!("unknown handler: {handler}"))?;
    reject_undeclared_inputs(handler, input, spec)?;
    match handler {
        "minibox::container::run" => {
            let image = str_field(input, "image")?;
            let tag = opt_str_field(input, "tag")?;
            let command = str_array_field(input, "command")?.unwrap_or_default();
            let memory_limit_bytes = opt_u64_field(input, "memory_limit_bytes")?;
            let cpu_weight = opt_u64_field(input, "cpu_weight")?;
            let env = str_array_field(input, "env")?.unwrap_or_default();
            let name = opt_str_field(input, "name")?;
            let platform = opt_str_field(input, "platform")?;
            let privileged = opt_bool_field(input, "privileged")?.unwrap_or(false);
            let mounts = parse_mounts(input)?;

            Ok(DaemonRequest::Run {
                image,
                tag,
                command,
                memory_limit_bytes,
                cpu_weight,
                ephemeral: false,
                network: None,
                mounts,
                privileged,
                env,
                name,
                tty: false,
                entrypoint: None,
                user: None,
                auto_remove: false,
                priority: None,
                urgency: None,
                execution_context: None,
                platform,
                cgroup_parent: None,
            })
        }

        "minibox::container::stop" => {
            let id = str_field(input, "id")?;
            Ok(DaemonRequest::Stop { id })
        }

        "minibox::container::rm" => {
            let id = str_field(input, "id")?;
            Ok(DaemonRequest::Remove { id })
        }

        "minibox::container::pause" => {
            let id = str_field(input, "id")?;
            Ok(DaemonRequest::PauseContainer { id })
        }

        "minibox::container::resume" => {
            let id = str_field(input, "id")?;
            Ok(DaemonRequest::ResumeContainer { id })
        }

        "minibox::container::exec" => {
            let container_id = str_field(input, "id")?;
            let cmd = str_array_field(input, "command")?
                .ok_or_else(|| anyhow::anyhow!("exec requires 'command' array"))?;
            let env = str_array_field(input, "env")?.unwrap_or_default();
            let tty = opt_bool_field(input, "tty")?.unwrap_or(false);
            Ok(DaemonRequest::Exec {
                container_id,
                cmd,
                env,
                tty,
                working_dir: None,
                user: None,
            })
        }

        "minibox::container::ps" => Ok(DaemonRequest::List),

        "minibox::container::logs" => {
            let container_id = str_field(input, "id")?;
            Ok(DaemonRequest::ContainerLogs {
                container_id,
                follow: false,
            })
        }

        "minibox::image::pull" => {
            let image = str_field(input, "image")?;
            let tag = opt_str_field(input, "tag")?;
            let platform = opt_str_field(input, "platform")?;
            Ok(DaemonRequest::Pull {
                image,
                tag,
                platform,
            })
        }

        "minibox::image::build" => {
            let context_path = str_field(input, "context_path")?;
            let tag = if input["tag"].is_null() {
                "latest".to_string()
            } else {
                str_field(input, "tag")?
            };
            let dockerfile =
                opt_str_field(input, "dockerfile")?.unwrap_or_else(|| "FROM scratch".to_string());
            Ok(DaemonRequest::Build {
                dockerfile,
                context_path,
                tag,
                build_args: vec![],
                no_cache: false,
            })
        }

        "minibox::image::push" => {
            let image_ref = str_field(input, "image")?;
            Ok(DaemonRequest::Push {
                image_ref,
                credentials: minibox_core::protocol::PushCredentials::Anonymous,
            })
        }

        "minibox::image::ls" => Ok(DaemonRequest::ListImages),

        "minibox::image::rm" => {
            let image_ref = str_field(input, "image_ref")?;
            Ok(DaemonRequest::RemoveImage { image_ref })
        }

        other => anyhow::bail!("unknown handler: {other}"),
    }
}

// ── Input extraction helpers ───────────────────────────────────────────────────

/// Rejects any input field the handler does not declare.
///
/// Enforcing the declared set as the *complete* contract is what stops a caller
/// from supplying a field the plugin will quietly drop. Without this, a field
/// like an image push `target` could be accepted and discarded, leaving the
/// caller to believe the capability was applied.
fn reject_undeclared_inputs(handler: &str, input: &Value, spec: &HandlerSpec) -> Result<()> {
    let Some(object) = input.as_object() else {
        anyhow::bail!("{handler}: input must be a JSON object");
    };
    let accepted: Vec<&str> = if spec.inputs.is_empty() {
        vec!["<none>"]
    } else {
        spec.inputs.iter().map(|i| i.name).collect()
    };
    for key in object.keys() {
        if !spec.inputs.iter().any(|i| i.name == key) {
            anyhow::bail!(
                "{handler}: unsupported input field '{key}' (accepted fields: {})",
                accepted.join(", ")
            );
        }
    }
    Ok(())
}

/// Parses and validates bind mounts from a handler input object.
pub fn parse_mounts(v: &Value) -> Result<Vec<BindMount>> {
    let Some(arr) = v["mounts"].as_array() else {
        return Ok(vec![]);
    };
    arr.iter()
        .enumerate()
        .map(|(i, entry)| {
            let host_path = entry["host_path"]
                .as_str()
                .map(std::path::PathBuf::from)
                .ok_or_else(|| {
                    anyhow::anyhow!("mounts[{i}]: 'host_path' must be a non-null string")
                })?;
            if !host_path.is_absolute()
                || host_path
                    .components()
                    .any(|c| c == std::path::Component::ParentDir)
            {
                anyhow::bail!("mounts[{i}]: host_path must be absolute with no '..' components");
            }
            let container_path = entry["container_path"]
                .as_str()
                .map(std::path::PathBuf::from)
                .ok_or_else(|| {
                    anyhow::anyhow!("mounts[{i}]: 'container_path' must be a non-null string")
                })?;
            if !container_path.is_absolute()
                || container_path
                    .components()
                    .any(|c| c == std::path::Component::ParentDir)
            {
                anyhow::bail!(
                    "mounts[{i}]: container_path must be absolute with no '..' components"
                );
            }
            let read_only = entry["read_only"].as_bool().unwrap_or(false);
            Ok(BindMount {
                host_path,
                container_path,
                read_only,
            })
        })
        .collect()
}

/// Extracts a required string field from a handler input object.
pub fn str_field(v: &Value, key: &str) -> Result<String> {
    v[key]
        .as_str()
        .map(std::string::ToString::to_string)
        .ok_or_else(|| anyhow::anyhow!("missing or non-string field '{key}'"))
}

/// Extracts an optional string field from a handler input object.
///
/// A key that is present but not a string is an error rather than a silent
/// `None`, so a malformed value is never dropped as if it were absent.
pub fn opt_str_field(v: &Value, key: &str) -> Result<Option<String>> {
    if v[key].is_null() {
        return Ok(None);
    }
    v[key]
        .as_str()
        .map(|s| Some(std::string::ToString::to_string(s)))
        .ok_or_else(|| anyhow::anyhow!("field '{key}' must be a string"))
}

/// Extracts an optional unsigned integer field from a handler input object.
///
/// A key that is present but not a `u64` is an error rather than a silent
/// `None`, so a malformed value is never dropped as if it were absent.
pub fn opt_u64_field(v: &Value, key: &str) -> Result<Option<u64>> {
    if v[key].is_null() {
        return Ok(None);
    }
    v[key]
        .as_u64()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("field '{key}' must be a non-negative integer"))
}

/// Extracts an optional boolean field from a handler input object.
///
/// A key that is present but not a boolean is an error rather than a silent
/// `None`, so a malformed value is never dropped as if it were absent.
pub fn opt_bool_field(v: &Value, key: &str) -> Result<Option<bool>> {
    if v[key].is_null() {
        return Ok(None);
    }
    v[key]
        .as_bool()
        .map(Some)
        .ok_or_else(|| anyhow::anyhow!("field '{key}' must be a boolean"))
}

/// Extracts an optional array of string values from a handler input object.
///
/// A key that is present but is not an array of strings is an error: the
/// previous lenient form filtered non-string elements out, which silently
/// dropped caller-supplied entries.
pub fn str_array_field(v: &Value, key: &str) -> Result<Option<Vec<String>>> {
    if v[key].is_null() {
        return Ok(None);
    }
    let array = v[key]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("field '{key}' must be an array of strings"))?;
    array
        .iter()
        .enumerate()
        .map(|(i, element)| {
            element
                .as_str()
                .map(std::string::ToString::to_string)
                .ok_or_else(|| anyhow::anyhow!("field '{key}': element {i} must be a string"))
        })
        .collect::<Result<Vec<String>>>()
        .map(Some)
}

/// Process a single request line and produce a response.
pub async fn process_request(request: Request) -> Option<Response> {
    match request {
        Request::Declare => Some(Response::Declare {
            handlers: handler_decls(),
        }),

        Request::Invoke { handler, input } => match dispatch(&handler, input).await {
            Ok(output) => Some(Response::InvokeOk { output }),
            Err(e) => {
                warn!(handler = %handler, error = %e, "handler invocation failed");
                Some(Response::InvokeErr {
                    error: e.to_string(),
                })
            }
        },

        Request::Shutdown => None, // Caller handles shutdown ack + break
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── build_request mapping tests ────────────────────────────────────────────

    #[test]
    fn build_request_container_run_minimal() {
        let input = json!({"image": "alpine:latest", "command": ["/bin/sh"]});
        let req = build_request("minibox::container::run", &input)
            .expect("build_request for minimal run input");
        match req {
            DaemonRequest::Run { image, command, .. } => {
                assert_eq!(image, "alpine:latest");
                assert_eq!(command, vec!["/bin/sh"]);
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn build_request_run_maps_privileged_true() {
        let req = build_request(
            "minibox::container::run",
            &json!({"image": "alpine", "privileged": true}),
        )
        .expect("build_request");
        let DaemonRequest::Run { privileged, .. } = req else {
            panic!("expected Run");
        };
        assert!(privileged, "privileged: true must pass through to Run");
    }

    #[test]
    fn build_request_run_defaults_privileged_false() {
        let req = build_request("minibox::container::run", &json!({"image": "alpine"}))
            .expect("build_request");
        let DaemonRequest::Run { privileged, .. } = req else {
            panic!("expected Run");
        };
        assert!(!privileged, "privileged must default to false when absent");
    }

    #[test]
    fn build_request_container_stop() {
        let input = json!({"id": "abc123"});
        let req = build_request("minibox::container::stop", &input).unwrap();
        assert!(matches!(req, DaemonRequest::Stop { id } if id == "abc123"));
    }

    #[test]
    fn build_request_container_rm() {
        let input = json!({"id": "abc123"});
        let req = build_request("minibox::container::rm", &input).unwrap();
        assert!(matches!(req, DaemonRequest::Remove { id } if id == "abc123"));
    }

    #[test]
    fn build_request_container_ps() {
        let req = build_request("minibox::container::ps", &json!({})).unwrap();
        assert!(matches!(req, DaemonRequest::List));
    }

    #[test]
    fn build_request_container_logs() {
        let input = json!({"id": "abc123"});
        let req = build_request("minibox::container::logs", &input).unwrap();
        assert!(
            matches!(req, DaemonRequest::ContainerLogs { container_id, .. } if container_id == "abc123")
        );
    }

    #[test]
    fn build_request_image_pull() {
        let input = json!({"image": "ubuntu"});
        let req = build_request("minibox::image::pull", &input).unwrap();
        assert!(matches!(req, DaemonRequest::Pull { image, .. } if image == "ubuntu"));
    }

    #[test]
    fn build_request_image_build() {
        let input = json!({"context_path": "/tmp/ctx", "tag": "myapp:latest"});
        let req = build_request("minibox::image::build", &input).unwrap();
        match req {
            DaemonRequest::Build {
                context_path, tag, ..
            } => {
                assert_eq!(context_path, "/tmp/ctx");
                assert_eq!(tag, "myapp:latest");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn build_request_image_push() {
        let input = json!({"image": "myapp:latest"});
        let req = build_request("minibox::image::push", &input).unwrap();
        match req {
            DaemonRequest::Push { image_ref, .. } => {
                assert_eq!(image_ref, "myapp:latest");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn build_request_unknown_handler_returns_err() {
        let result = build_request("minibox::unknown::handler", &json!({}));
        assert!(result.is_err());
        let msg = result.expect_err("unknown handler must fail").to_string();
        assert!(msg.contains("unknown handler"));
    }

    #[test]
    fn build_request_missing_required_field_returns_err() {
        // "id" is required for stop
        let result = build_request("minibox::container::stop", &json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn build_request_container_run_with_mounts() {
        let input = json!({
            "image": "alpine:latest",
            "command": ["/bin/sh"],
            "mounts": [
                {
                    "host_path": "/tmp/data",
                    "container_path": "/data",
                    "read_only": true
                },
                {
                    "host_path": "/home/user/src",
                    "container_path": "/src",
                    "read_only": false
                }
            ]
        });
        let req =
            build_request("minibox::container::run", &input).expect("build_request must succeed");
        match req {
            DaemonRequest::Run { mounts, .. } => {
                assert_eq!(mounts.len(), 2, "expected 2 mounts");
                assert_eq!(mounts[0].host_path, std::path::PathBuf::from("/tmp/data"));
                assert_eq!(mounts[0].container_path, std::path::PathBuf::from("/data"));
                assert!(mounts[0].read_only, "first mount must be read_only");
                assert_eq!(
                    mounts[1].host_path,
                    std::path::PathBuf::from("/home/user/src")
                );
                assert!(!mounts[1].read_only, "second mount must not be read_only");
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn build_request_container_run_empty_mounts_when_absent() {
        let input = json!({"image": "alpine:latest", "command": ["/bin/sh"]});
        let req =
            build_request("minibox::container::run", &input).expect("build_request must succeed");
        match req {
            DaemonRequest::Run { mounts, .. } => {
                assert!(mounts.is_empty(), "mounts must be empty when not provided");
            }
            other => panic!("unexpected request: {other:?}"),
        }
    }

    #[test]
    fn build_request_run_mount_missing_host_path_returns_err() {
        let input = json!({
            "image": "alpine:latest",
            "command": ["/bin/sh"],
            "mounts": [
                {
                    "container_path": "/data",
                    "read_only": false
                }
            ]
        });
        let result = build_request("minibox::container::run", &input);
        assert!(result.is_err(), "expected Err for mount missing host_path");
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("host_path"),
            "error message should mention 'host_path', got: {msg}"
        );
    }

    #[test]
    fn handler_decls_covers_all_handlers() {
        let decls = handler_decls();
        assert_eq!(decls.len(), 13, "expected 13 handler declarations");
        let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"minibox::container::run"));
        assert!(names.contains(&"minibox::container::stop"));
        assert!(names.contains(&"minibox::container::rm"));
        assert!(names.contains(&"minibox::container::exec"));
        assert!(names.contains(&"minibox::container::ps"));
        assert!(names.contains(&"minibox::container::logs"));
        assert!(names.contains(&"minibox::image::pull"));
        assert!(names.contains(&"minibox::image::build"));
        assert!(names.contains(&"minibox::image::push"));
    }

    // ── pause / resume ─────────────────────────────────────────────────────────

    #[test]
    fn build_request_container_pause() {
        let input = json!({"id": "abc123"});
        let req = build_request("minibox::container::pause", &input)
            .expect("build_request must succeed for pause");
        assert!(
            matches!(req, DaemonRequest::PauseContainer { id } if id == "abc123"),
            "expected PauseContainer{{id: \"abc123\"}}, got unexpected variant"
        );
    }

    #[test]
    fn build_request_container_resume() {
        let input = json!({"id": "abc123"});
        let req = build_request("minibox::container::resume", &input)
            .expect("build_request must succeed for resume");
        assert!(
            matches!(req, DaemonRequest::ResumeContainer { id } if id == "abc123"),
            "expected ResumeContainer{{id: \"abc123\"}}, got unexpected variant"
        );
    }

    #[test]
    fn build_request_container_pause_missing_id_returns_err() {
        let result = build_request("minibox::container::pause", &json!({}));
        assert!(result.is_err(), "pause without id must return Err");
    }

    #[test]
    fn build_request_container_resume_missing_id_returns_err() {
        let result = build_request("minibox::container::resume", &json!({}));
        assert!(result.is_err(), "resume without id must return Err");
    }

    #[test]
    fn handler_decls_covers_pause_and_resume() {
        let decls = handler_decls();
        let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        assert!(
            names.contains(&"minibox::container::pause"),
            "handler_decls must include pause"
        );
        assert!(
            names.contains(&"minibox::container::resume"),
            "handler_decls must include resume"
        );
    }

    // ── image::ls / image::rm ──────────────────────────────────────────────────

    #[test]
    fn build_request_image_ls() {
        let req = build_request("minibox::image::ls", &json!({}))
            .expect("image::ls must succeed with empty input");
        assert!(
            matches!(req, DaemonRequest::ListImages),
            "expected DaemonRequest::ListImages, got: {req:?}"
        );
    }

    #[test]
    fn build_request_image_rm() {
        let input = json!({"image_ref": "alpine:latest"});
        let req = build_request("minibox::image::rm", &input)
            .expect("image::rm must succeed with image_ref");
        assert!(
            matches!(req, DaemonRequest::RemoveImage { ref image_ref } if image_ref == "alpine:latest"),
            "expected DaemonRequest::RemoveImage{{image_ref: \"alpine:latest\"}}, got: {req:?}"
        );
    }

    #[test]
    fn build_request_image_rm_missing_image_ref_returns_err() {
        let result = build_request("minibox::image::rm", &json!({}));
        assert!(
            result.is_err(),
            "image::rm without image_ref must return Err"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("image_ref"),
            "error message should mention 'image_ref', got: {msg}"
        );
    }

    // ── parse_mounts path-traversal rejection ────────────────────────────────

    #[test]
    fn parse_mounts_rejects_relative_host_path() {
        let input = json!({
            "mounts": [{"host_path": "relative/path", "container_path": "/data"}]
        });
        let result = parse_mounts(&input);
        let msg = result
            .expect_err("relative host_path must fail")
            .to_string();
        assert!(
            msg.contains("host_path"),
            "error should mention host_path: {msg}"
        );
    }

    #[test]
    fn parse_mounts_rejects_dotdot_in_host_path() {
        let input = json!({
            "mounts": [{"host_path": "/tmp/../etc/shadow", "container_path": "/data"}]
        });
        let result = parse_mounts(&input);
        let msg = result.expect_err(".. in host_path must fail").to_string();
        assert!(
            msg.contains("host_path"),
            "error should mention host_path: {msg}"
        );
    }

    #[test]
    fn parse_mounts_rejects_relative_container_path() {
        let input = json!({
            "mounts": [{"host_path": "/tmp/data", "container_path": "data"}]
        });
        let result = parse_mounts(&input);
        let msg = result
            .expect_err("relative container_path must fail")
            .to_string();
        assert!(
            msg.contains("container_path"),
            "error should mention container_path: {msg}"
        );
    }

    #[test]
    fn parse_mounts_rejects_dotdot_in_container_path() {
        let input = json!({
            "mounts": [{"host_path": "/tmp/data", "container_path": "/data/../etc"}]
        });
        let result = parse_mounts(&input);
        let msg = result
            .expect_err(".. in container_path must fail")
            .to_string();
        assert!(
            msg.contains("container_path"),
            "error should mention container_path: {msg}"
        );
    }

    #[test]
    fn handler_decls_covers_image_ls_and_image_rm() {
        let decls = handler_decls();
        assert_eq!(decls.len(), 13, "expected 13 handler declarations");
        let names: Vec<&str> = decls.iter().map(|d| d.name.as_str()).collect();
        assert!(
            names.contains(&"minibox::image::ls"),
            "handler_decls must include image::ls"
        );
        assert!(
            names.contains(&"minibox::image::rm"),
            "handler_decls must include image::rm"
        );
    }

    // ── image::push target resolution (issue #518) ───────────────────────────

    /// A Crux planner reads the `Declare` payload to decide what it can ask for.
    /// Advertising a push `target` the plugin cannot honor makes it plan a
    /// retag-and-push to a foreign registry that silently never happens, so the
    /// declared surface must not claim one.
    #[test]
    fn image_push_decl_does_not_advertise_unimplemented_target() {
        let decl = handler_decls()
            .into_iter()
            .find(|d| d.name == "minibox::image::push")
            .expect("image::push must be declared");
        assert!(
            !decl.description.contains("target"),
            "image::push must not advertise a separate push target it cannot honor, \
             got description: {}",
            decl.description
        );
    }

    /// The target is not merely unadvertised — supplying one is an error. Quietly
    /// dropping it would leave the caller believing a foreign-registry push ran.
    #[test]
    fn image_push_rejects_target_instead_of_discarding_it() {
        let err = build_request(
            "minibox::image::push",
            &json!({"image": "myapp:latest", "target": "ghcr.io/joe/myapp:v1"}),
        )
        .expect_err("an unimplemented push target must be rejected, not discarded");
        let msg = err.to_string();
        assert!(
            msg.contains("target"),
            "error must name the rejected 'target' field, got: {msg}"
        );
    }

    #[test]
    fn image_push_rejects_credentials_it_cannot_supply() {
        let err = build_request(
            "minibox::image::push",
            &json!({
                "image": "myapp:latest",
                "credentials": {"type": "Basic", "username": "u", "password": "p"}
            }),
        )
        .expect_err("push credentials are not bound by the plugin, so they must be rejected");
        let msg = err.to_string();
        assert!(
            msg.contains("credentials"),
            "error must name the rejected 'credentials' field, got: {msg}"
        );
    }

    // ── Input parity with the daemon protocol (issue #518) ───────────────────

    /// Ground truth for one handler, audited against the corresponding
    /// [`DaemonRequest`] variant in `minibox_core::protocol`.
    ///
    /// * `required` / `optional` — the exact input contract `handler_decls()`
    ///   must advertise. Every one of these is bound into the outgoing request.
    /// * `must_reject` — fields the protocol provides (or a caller would
    ///   reasonably guess from it) that this plugin does not implement. These
    ///   must be rejected outright, never silently discarded.
    struct ParityRow {
        /// Namespaced handler name.
        handler: &'static str,
        /// Inputs without which the handler must fail.
        required: &'static [&'static str],
        /// Inputs that are honored when present.
        optional: &'static [&'static str],
        /// Protocol fields the plugin does not implement.
        must_reject: &'static [&'static str],
    }

    /// One row per declared handler, in `handler_decls()` order.
    const PARITY: &[ParityRow] = &[
        ParityRow {
            handler: "minibox::container::run",
            required: &["image"],
            optional: &[
                "tag",
                "command",
                "env",
                "mounts",
                "memory_limit_bytes",
                "cpu_weight",
                "name",
                "platform",
                "privileged",
            ],
            // `DaemonRequest::Run` also carries these; the plugin hardcodes them.
            must_reject: &[
                "ephemeral",
                "network",
                "tty",
                "entrypoint",
                "user",
                "auto_remove",
                "priority",
                "urgency",
                "execution_context",
                "cgroup_parent",
            ],
        },
        ParityRow {
            handler: "minibox::container::stop",
            required: &["id"],
            optional: &[],
            must_reject: &[],
        },
        ParityRow {
            handler: "minibox::container::pause",
            required: &["id"],
            optional: &[],
            must_reject: &[],
        },
        ParityRow {
            handler: "minibox::container::resume",
            required: &["id"],
            optional: &[],
            must_reject: &[],
        },
        ParityRow {
            handler: "minibox::container::rm",
            required: &["id"],
            optional: &[],
            must_reject: &[],
        },
        ParityRow {
            handler: "minibox::container::exec",
            required: &["id", "command"],
            optional: &["env", "tty"],
            // Also covers the protocol's own `container_id`/`cmd` spellings:
            // a caller guessing those must get a clear error, not a "missing id".
            must_reject: &["working_dir", "user", "container_id", "cmd"],
        },
        ParityRow {
            handler: "minibox::container::ps",
            required: &[],
            optional: &[],
            must_reject: &[],
        },
        ParityRow {
            handler: "minibox::container::logs",
            required: &["id"],
            optional: &[],
            must_reject: &["follow"],
        },
        ParityRow {
            handler: "minibox::image::pull",
            required: &["image"],
            optional: &["tag", "platform"],
            must_reject: &[],
        },
        ParityRow {
            handler: "minibox::image::build",
            required: &["context_path"],
            optional: &["tag", "dockerfile"],
            must_reject: &["build_args", "no_cache"],
        },
        ParityRow {
            handler: "minibox::image::push",
            required: &["image"],
            optional: &[],
            // The `target` the plugin used to advertise and silently drop, plus
            // the credentials it cannot supply.
            must_reject: &["target", "credentials"],
        },
        ParityRow {
            handler: "minibox::image::ls",
            required: &[],
            optional: &[],
            must_reject: &[],
        },
        ParityRow {
            handler: "minibox::image::rm",
            required: &["image_ref"],
            optional: &[],
            must_reject: &[],
        },
    ];

    /// A representative, type-correct value for an input field name.
    ///
    /// Used to prove a declared field is genuinely accepted and bound, and that
    /// an undeclared one is rejected for being undeclared rather than for
    /// having the wrong type.
    fn sample_value(field: &str) -> Value {
        match field {
            "command" => json!(["/bin/sh"]),
            "env" => json!(["KEY=VALUE"]),
            "mounts" => json!([{
                "host_path": "/tmp/data",
                "container_path": "/data",
                "read_only": true
            }]),
            "memory_limit_bytes" | "cpu_weight" | "build_args" => json!(1024_u64),
            "privileged" | "tty" | "no_cache" | "follow" | "auto_remove" | "ephemeral" => {
                json!(true)
            }
            "network" | "priority" | "urgency" | "execution_context" | "credentials" => {
                json!("bridge")
            }
            "cgroup_parent" | "working_dir" | "entrypoint" | "user" | "target" | "cmd" => {
                json!("/some/value")
            }
            _ => json!("sample"),
        }
    }

    /// Builds an input object carrying every name in `fields`.
    fn input_of(fields: &[&str]) -> Value {
        let mut object = serde_json::Map::new();
        for name in fields {
            object.insert((*name).to_string(), sample_value(name));
        }
        Value::Object(object)
    }

    /// The audited contract for `handler`, looked up from the table.
    fn parity_row(handler: &str) -> &'static ParityRow {
        PARITY
            .iter()
            .find(|row| row.handler == handler)
            .unwrap_or_else(|| panic!("no parity row audited for handler '{handler}'"))
    }

    /// Every declared handler is audited, and every audited handler is declared.
    ///
    /// Without this, adding a handler to `handler_decls()` without auditing it
    /// would silently escape the parity assertions below.
    #[test]
    fn parity_table_covers_every_declared_handler() {
        let declared: Vec<String> = handler_decls().into_iter().map(|d| d.name).collect();
        let audited: Vec<&str> = PARITY.iter().map(|row| row.handler).collect();

        for name in &declared {
            assert!(
                audited.contains(&name.as_str()),
                "handler '{name}' is declared but missing a PARITY row"
            );
        }
        for name in &audited {
            assert!(
                declared.iter().any(|d| d == name),
                "handler '{name}' has a PARITY row but is not declared"
            );
        }
        assert_eq!(declared.len(), audited.len());
    }

    /// Each handler's declared inputs equal the audited protocol contract.
    ///
    /// This is the table over `handler_decls()` the issue asks for: a declared
    /// handler that lists a different field set than the audit — in either
    /// direction — fails here.
    #[test]
    fn handler_decls_declares_exactly_the_audited_protocol_inputs() {
        for decl in handler_decls() {
            let row = parity_row(&decl.name);
            let declared_required: Vec<&str> = decl
                .inputs
                .iter()
                .filter(|i| i.required)
                .map(|i| i.name.as_str())
                .collect();
            let declared_optional: Vec<&str> = decl
                .inputs
                .iter()
                .filter(|i| !i.required)
                .map(|i| i.name.as_str())
                .collect();

            assert_eq!(
                declared_required, row.required,
                "{}: required inputs drifted from the audited protocol shape",
                decl.name
            );
            assert_eq!(
                declared_optional, row.optional,
                "{}: optional inputs drifted from the audited protocol shape",
                decl.name
            );

            for rejected in row.must_reject {
                assert!(
                    !decl.inputs.iter().any(|i| i.name == *rejected),
                    "{}: must not advertise '{rejected}' — it is not implemented",
                    decl.name
                );
            }
        }
    }

    /// The `Input: {…}` list a planner reads is rendered from the enforced
    /// contract, so the two cannot describe different things.
    #[test]
    fn handler_description_lists_exactly_the_declared_inputs() {
        for decl in handler_decls() {
            let expected = if decl.inputs.is_empty() {
                "Input: {}".to_string()
            } else {
                let names: Vec<&str> = decl.inputs.iter().map(|i| i.name.as_str()).collect();
                format!("Input: {{{}}}", names.join(", "))
            };
            assert!(
                decl.description.contains(&expected),
                "{}: description must contain '{expected}', got: {}",
                decl.name,
                decl.description
            );
        }
    }

    /// Supplying only the required inputs is enough to build a request.
    #[test]
    fn every_handler_builds_from_required_inputs_alone() {
        for row in PARITY {
            let input = input_of(row.required);
            let result = build_request(row.handler, &input);
            assert!(
                result.is_ok(),
                "{}: required inputs {:?} must build a request, got: {:?}",
                row.handler,
                row.required,
                result.expect_err("expected success")
            );
        }
    }

    /// Omitting any single required input fails — the contract's `required`
    /// flag is load-bearing, not decorative.
    #[test]
    fn every_required_input_is_enforced() {
        for row in PARITY {
            for missing in row.required {
                let present: Vec<&str> = row
                    .required
                    .iter()
                    .copied()
                    .filter(|name| name != missing)
                    .collect();
                let result = build_request(row.handler, &input_of(&present));
                assert!(
                    result.is_err(),
                    "{}: must fail without required input '{missing}'",
                    row.handler
                );
            }
        }
    }

    /// Every declared optional input is genuinely accepted when supplied, so
    /// declaring a field always corresponds to binding it.
    #[test]
    fn every_declared_optional_input_is_accepted() {
        for row in PARITY {
            for optional in row.optional {
                let mut fields: Vec<&str> = row.required.to_vec();
                fields.push(optional);
                let result = build_request(row.handler, &input_of(&fields));
                assert!(
                    result.is_ok(),
                    "{}: declared optional input '{optional}' must be accepted, got: {:?}",
                    row.handler,
                    result.expect_err("expected success")
                );
            }
        }
    }

    /// The negative half of the contract: protocol fields the plugin does not
    /// implement are rejected, not silently dropped.
    #[test]
    fn every_unimplemented_protocol_input_is_rejected() {
        for row in PARITY {
            for rejected in row.must_reject {
                let mut fields: Vec<&str> = row.required.to_vec();
                fields.push(rejected);
                let result = build_request(row.handler, &input_of(&fields));
                let err = result.expect_err(&format!(
                    "{}: unimplemented input '{rejected}' must be rejected",
                    row.handler
                ));
                let msg = err.to_string();
                assert!(
                    msg.contains(&format!("'{rejected}'")),
                    "{}: error must name the rejected field '{rejected}', got: {msg}",
                    row.handler
                );
            }
        }
    }

    /// Uniform negative coverage: no handler accepts a field it never declared,
    /// so an unknown or misspelled key is reported rather than ignored.
    #[test]
    fn every_handler_rejects_an_undeclared_field() {
        for row in PARITY {
            let mut fields: Vec<&str> = row.required.to_vec();
            fields.push("totally_undeclared_field");
            let err = build_request(row.handler, &input_of(&fields))
                .expect_err(&format!("{}: must reject an undeclared field", row.handler));
            let msg = err.to_string();
            assert!(
                msg.contains("totally_undeclared_field"),
                "{}: error must name the undeclared field, got: {msg}",
                row.handler
            );
            assert!(
                msg.contains(row.handler),
                "{}: error must name the handler, got: {msg}",
                row.handler
            );
        }
    }

    /// A zero-input handler rejects any field rather than tolerating one.
    #[test]
    fn zero_input_handlers_reject_any_field() {
        for handler in ["minibox::container::ps", "minibox::image::ls"] {
            let err = build_request(handler, &json!({"all": true})).expect_err(&format!(
                "{handler} declares no inputs and must reject 'all'"
            ));
            assert!(
                err.to_string().contains("'all'"),
                "{handler}: error must name the rejected field, got: {err}"
            );
        }
    }

    /// A non-object payload is a contract violation, not an empty request.
    #[test]
    fn non_object_input_is_rejected() {
        for payload in [json!("a string"), json!([1, 2, 3]), json!(null)] {
            let err = build_request("minibox::container::ps", &payload)
                .expect_err("non-object input must be rejected");
            assert!(
                err.to_string().contains("must be a JSON object"),
                "error must explain the object requirement, got: {err}"
            );
        }
    }

    /// Malformed values are rejected rather than treated as absent — the lenient
    /// form silently dropped non-string array elements.
    #[test]
    fn malformed_values_are_rejected_not_dropped() {
        let cases: &[(&str, Value, &str)] = &[
            (
                "minibox::container::run",
                json!({"image": "alpine", "command": ["/bin/sh", 7]}),
                "element 1",
            ),
            (
                "minibox::container::run",
                json!({"image": "alpine", "cpu_weight": "500"}),
                "non-negative integer",
            ),
            (
                "minibox::container::run",
                json!({"image": "alpine", "privileged": "yes"}),
                "boolean",
            ),
            (
                "minibox::image::pull",
                json!({"image": "alpine", "tag": 22}),
                "must be a string",
            ),
            (
                "minibox::container::run",
                json!({"image": "alpine", "command": "/bin/sh"}),
                "array of strings",
            ),
        ];
        for (handler, input, expected) in cases {
            let err = build_request(handler, input).expect_err("malformed input must be rejected");
            let msg = err.to_string();
            assert!(
                msg.contains(expected),
                "{handler}: error must mention '{expected}', got: {msg}"
            );
        }
    }

    // ── Declared fields are bound, not just accepted ────────────────────────

    #[test]
    fn build_request_run_binds_tag() {
        let req = build_request(
            "minibox::container::run",
            &json!({"image": "ubuntu", "tag": "22.04"}),
        )
        .expect("build_request");
        let DaemonRequest::Run { tag, .. } = req else {
            panic!("expected Run");
        };
        assert_eq!(tag, Some("22.04".to_string()), "tag must be bound");
    }

    #[test]
    fn build_request_pull_binds_tag_and_platform() {
        let req = build_request(
            "minibox::image::pull",
            &json!({"image": "ubuntu", "tag": "22.04", "platform": "linux/arm64"}),
        )
        .expect("build_request");
        let DaemonRequest::Pull { tag, platform, .. } = req else {
            panic!("expected Pull");
        };
        assert_eq!(tag, Some("22.04".to_string()), "tag must be bound");
        assert_eq!(
            platform,
            Some("linux/arm64".to_string()),
            "platform must be bound"
        );
    }

    #[test]
    fn build_request_build_binds_dockerfile() {
        let req = build_request(
            "minibox::image::build",
            &json!({"context_path": "/tmp/ctx", "dockerfile": "FROM alpine"}),
        )
        .expect("build_request");
        let DaemonRequest::Build {
            dockerfile, tag, ..
        } = req
        else {
            panic!("expected Build");
        };
        assert_eq!(dockerfile, "FROM alpine", "dockerfile must be bound");
        assert_eq!(tag, "latest", "tag must still default to latest");
    }

    #[test]
    fn build_request_exec_binds_tty_and_env() {
        let req = build_request(
            "minibox::container::exec",
            &json!({"id": "abc", "command": ["ls"], "env": ["A=1"], "tty": true}),
        )
        .expect("build_request");
        let DaemonRequest::Exec { cmd, env, tty, .. } = req else {
            panic!("expected Exec");
        };
        assert_eq!(cmd, vec!["ls".to_string()]);
        assert_eq!(env, vec!["A=1".to_string()]);
        assert!(tty, "tty must be bound");
    }

    /// The `Declare` payload carries the machine-readable contract, so a host can
    /// validate input before dispatch.
    #[test]
    fn declare_payload_serializes_declared_inputs() {
        let decls = handler_decls();
        let encoded = serde_json::to_value(&decls).expect("serialize decls");
        let push = encoded
            .as_array()
            .expect("decls array")
            .iter()
            .find(|d| d["name"] == json!("minibox::image::push"))
            .expect("push entry");
        let inputs = push["inputs"].as_array().expect("inputs array");
        assert_eq!(inputs.len(), 1, "push declares exactly one input");
        assert_eq!(inputs[0]["name"], json!("image"));
        assert_eq!(inputs[0]["required"], json!(true));
    }
}
