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

/// A single input field a handler accepts.
///
/// Declared inputs form the handler's complete input contract: every declared
/// field is bound into the outgoing daemon request, and any field *not*
/// declared is rejected with an error rather than silently ignored. Keeping the
/// contract machine-readable (rather than only as prose in the description)
/// lets the host validate a payload before dispatch and lets the crate assert
/// the contract stays in sync with its own request builders.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HandlerInput {
    /// Field name as it must appear in the handler's `input` object.
    pub name: String,
    /// Whether the field must be present for the handler to succeed.
    pub required: bool,
}

/// A handler declared by a plugin.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HandlerDecl {
    /// Namespaced handler name, e.g. `github::create_issue`.
    pub name: String,
    /// One-line description for planner/help output.
    ///
    /// The `Input: {…}` portion is rendered from [`HandlerDecl::inputs`] so the
    /// human-readable surface can never drift from the enforced contract.
    pub description: String,
    /// Complete set of input fields this handler accepts.
    ///
    /// Defaults to empty for forward compatibility with hosts that predate the
    /// field and ignore it.
    #[serde(default)]
    pub inputs: Vec<HandlerInput>,
}
