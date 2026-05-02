//! minibox-crux-plugin — crux JSON-RPC plugin for minibox container operations.
//!
//! Implements the cruxx plugin stdio protocol:
//! - Reads `Request` (newline-delimited JSON) from stdin
//! - Writes `Response` (newline-delimited JSON) to stdout
//!
//! Exposes the minibox container API as crux handlers under the
//! `minibox::container::*` and `minibox::image::*` namespaces.

use anyhow::{Context, Result};
use cruxx_plugin::protocol::{HandlerDecl, Request, Response};
use minibox_core::client::{DaemonClient, default_socket_path};
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tracing::{debug, info, warn};

// ── Handler declarations ───────────────────────────────────────────────────────

/// All handlers exposed by this plugin, in declaration order.
fn handler_decls() -> Vec<HandlerDecl> {
    vec![
        HandlerDecl {
            name: "minibox::container::run".into(),
            description: "Create and start a container. Input: {image, command, env, mounts, \
                          memory_limit_bytes, cpu_weight, name, platform}"
                .into(),
        },
        HandlerDecl {
            name: "minibox::container::stop".into(),
            description: "Stop a running container. Input: {id}".into(),
        },
        HandlerDecl {
            name: "minibox::container::rm".into(),
            description: "Remove a stopped container. Input: {id}".into(),
        },
        HandlerDecl {
            name: "minibox::container::exec".into(),
            description: "Execute a command in a running container (Linux native only). \
                          Input: {id, command, env, tty}"
                .into(),
        },
        HandlerDecl {
            name: "minibox::container::ps".into(),
            description: "List all containers. Input: {}".into(),
        },
        HandlerDecl {
            name: "minibox::container::logs".into(),
            description: "Fetch logs for a container. Input: {id}".into(),
        },
        HandlerDecl {
            name: "minibox::image::pull".into(),
            description: "Pull an image from a registry. Input: {image}".into(),
        },
        HandlerDecl {
            name: "minibox::image::build".into(),
            description: "Build an image from a Dockerfile. Input: {context_path, tag}".into(),
        },
        HandlerDecl {
            name: "minibox::image::push".into(),
            description: "Push an image to a registry. Input: {image, target}".into(),
        },
    ]
}

// ── Handler dispatch ───────────────────────────────────────────────────────────

/// Route a handler invocation to the appropriate daemon request.
///
/// Returns `Ok(Value)` on success (the JSON response payload) or `Err` on
/// failure.  The caller is responsible for wrapping the result in
/// `Response::InvokeOk` / `Response::InvokeErr`.
async fn dispatch(handler: &str, input: Value) -> Result<Value> {
    let client = DaemonClient::with_socket(default_socket_path());

    let request = build_request(handler, &input)
        .with_context(|| format!("build_request for handler '{handler}'"))?;

    debug!(handler, "dispatching to daemon");

    let mut stream = client
        .call(request)
        .await
        .with_context(|| format!("daemon call for handler '{handler}'"))?;

    // Collect all responses until the stream closes.
    let mut responses: Vec<Value> = Vec::new();
    while let Some(resp) = stream.next().await.context("read daemon response")? {
        let is_terminal = matches!(
            &resp,
            DaemonResponse::Success { .. }
                | DaemonResponse::Error { .. }
                | DaemonResponse::ContainerStopped { .. }
                | DaemonResponse::ContainerCreated { .. }
                | DaemonResponse::ContainerList { .. }
        );
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
fn build_request(handler: &str, input: &Value) -> Result<DaemonRequest> {
    match handler {
        "minibox::container::run" => {
            let image = str_field(input, "image")?;
            let tag = opt_str_field(input, "tag");
            let command = str_array_field(input, "command").unwrap_or_default();
            let memory_limit_bytes = opt_u64_field(input, "memory_limit_bytes");
            let cpu_weight = opt_u64_field(input, "cpu_weight");
            let env = str_array_field(input, "env").unwrap_or_default();
            let name = opt_str_field(input, "name");
            let platform = opt_str_field(input, "platform");

            Ok(DaemonRequest::Run {
                image,
                tag,
                command,
                memory_limit_bytes,
                cpu_weight,
                ephemeral: false,
                network: None,
                mounts: vec![],
                privileged: false,
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

        "minibox::container::exec" => {
            let container_id = str_field(input, "id")?;
            let cmd = str_array_field(input, "command")
                .ok_or_else(|| anyhow::anyhow!("exec requires 'command' array"))?;
            let env = str_array_field(input, "env").unwrap_or_default();
            let tty = input["tty"].as_bool().unwrap_or(false);
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
            let tag = opt_str_field(input, "tag");
            let platform = opt_str_field(input, "platform");
            Ok(DaemonRequest::Pull {
                image,
                tag,
                platform,
            })
        }

        "minibox::image::build" => {
            let context_path = str_field(input, "context_path")?;
            let tag = str_field(input, "tag").unwrap_or_else(|_| "latest".into());
            let dockerfile =
                opt_str_field(input, "dockerfile").unwrap_or_else(|| "FROM scratch".into());
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

        other => anyhow::bail!("unknown handler: {other}"),
    }
}

// ── Input extraction helpers ───────────────────────────────────────────────────

fn str_field(v: &Value, key: &str) -> Result<String> {
    v[key]
        .as_str()
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow::anyhow!("missing or non-string field '{key}'"))
}

fn opt_str_field(v: &Value, key: &str) -> Option<String> {
    v[key].as_str().map(|s| s.to_string())
}

fn opt_u64_field(v: &Value, key: &str) -> Option<u64> {
    v[key].as_u64()
}

fn str_array_field(v: &Value, key: &str) -> Option<Vec<String>> {
    v[key].as_array().map(|arr| {
        arr.iter()
            .filter_map(|x| x.as_str().map(|s| s.to_string()))
            .collect()
    })
}

// ── Main loop ─────────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("minibox_crux_plugin=info".parse().unwrap()),
        )
        .init();

    info!("minibox-crux-plugin starting");

    let stdin = tokio::io::stdin();
    let stdout = tokio::io::stdout();
    let mut reader = BufReader::new(stdin).lines();
    let mut writer = tokio::io::BufWriter::new(stdout);

    while let Some(line) = reader.next_line().await.context("read stdin")? {
        let line = line.trim().to_string();
        if line.is_empty() {
            continue;
        }

        debug!(line = %line, "received request");

        let request: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                warn!(error = %e, "failed to parse request — skipping");
                continue;
            }
        };

        let response = match request {
            Request::Declare => Response::Declare {
                handlers: handler_decls(),
            },

            Request::Invoke { handler, input } => match dispatch(&handler, input).await {
                Ok(output) => Response::InvokeOk { output },
                Err(e) => {
                    warn!(handler = %handler, error = %e, "handler invocation failed");
                    Response::InvokeErr {
                        error: e.to_string(),
                    }
                }
            },

            Request::Shutdown => {
                info!("shutdown requested");
                let ack = serde_json::to_string(&Response::ShutdownAck)
                    .context("serialize ShutdownAck")?;
                writer
                    .write_all(format!("{ack}\n").as_bytes())
                    .await
                    .context("write ShutdownAck")?;
                writer.flush().await.context("flush")?;
                break;
            }
        };

        let encoded = serde_json::to_string(&response).context("serialize response")?;
        writer
            .write_all(format!("{encoded}\n").as_bytes())
            .await
            .context("write response")?;
        writer.flush().await.context("flush response")?;
    }

    info!("minibox-crux-plugin exiting");
    Ok(())
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
        let req = build_request("minibox::container::run", &input).unwrap();
        match req {
            DaemonRequest::Run { image, command, .. } => {
                assert_eq!(image, "alpine:latest");
                assert_eq!(command, vec!["/bin/sh"]);
            }
            other => panic!("unexpected request: {other:?}"),
        }
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
        assert!(result.unwrap_err().to_string().contains("unknown handler"));
    }

    #[test]
    fn build_request_missing_required_field_returns_err() {
        // "id" is required for stop
        let result = build_request("minibox::container::stop", &json!({}));
        assert!(result.is_err());
    }

    #[test]
    fn handler_decls_covers_all_nine_handlers() {
        let decls = handler_decls();
        assert_eq!(decls.len(), 9, "expected 9 handler declarations");
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
}
