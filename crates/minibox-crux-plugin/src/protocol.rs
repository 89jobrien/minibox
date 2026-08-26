//! JSON-RPC-like protocol for crux plugin communication.
//!
//! Messages are newline-delimited JSON on stdin/stdout.
//! Host sends `Request`, plugin replies with `Response`.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Host -> Plugin request.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "method", content = "params")]
pub enum Request {
    /// Ask the plugin to declare its handlers.
    Declare,
    /// Invoke a specific handler with input JSON.
    Invoke {
        /// Namespaced handler name to invoke.
        handler: String,
        /// JSON input payload for the handler.
        input: Value,
    },
    /// Ask the plugin to shut down gracefully.
    Shutdown,
}

/// Plugin -> Host response.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "status", content = "data")]
pub enum Response {
    /// Handler declarations returned by `Declare`.
    Declare {
        /// Handler declarations supported by this plugin.
        handlers: Vec<HandlerDecl>,
    },
    /// Successful handler invocation result.
    InvokeOk {
        /// JSON output returned by the invoked handler.
        output: Value,
    },
    /// Failed handler invocation.
    InvokeErr {
        /// Human-readable error returned by handler invocation.
        error: String,
    },
    /// Acknowledge shutdown.
    ShutdownAck,
}

/// A handler declared by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerDecl {
    /// Namespaced handler name, e.g. `github::create_issue`.
    pub name: String,
    /// One-line description for planner/help output.
    pub description: String,
}
