//! `test_run!` — construct a default `DaemonRequest::Run` for tests.
//!
//! Eliminates boilerplate when constructing `DaemonRequest::Run` in test code.
//! Every field gets a sensible default; override any subset with named arguments.
//!
//! # Examples
//!
//! ```rust,ignore
//! use minibox_macros::test_run;
//!
//! // All defaults
//! let req = test_run!();
//!
//! // Override specific fields
//! let req = test_run!(image: "ubuntu", tag: Some("22.04".to_string()));
//! let req = test_run!(env: vec!["FOO=bar".to_string()], privileged: true);
//! ```

/// Construct a `DaemonRequest::Run` with sensible test defaults.
///
/// All fields default to the simplest valid value (empty vecs, `None`, `false`).
/// Override any field by name. User-supplied fields shadow the defaults via
/// `let` bindings before the struct is constructed — no duplicate field issue.
///
/// The macro references `minibox_core::protocol::DaemonRequest` so it works
/// from any crate that depends on `minibox-core`.
#[macro_export]
macro_rules! test_run {
    ($($field:ident : $val:expr),* $(,)?) => {{
        // Defaults — user-supplied fields of the same name shadow these via
        // Rust variable shadowing. Types are inferred from the struct fields.
        #[allow(unused_variables)]
        let image = "alpine".to_string();
        #[allow(unused_variables)]
        let tag = None;
        #[allow(unused_variables)]
        let command = vec!["/bin/sh".to_string()];
        #[allow(unused_variables)]
        let memory_limit_bytes = None;
        #[allow(unused_variables)]
        let cpu_weight = None;
        #[allow(unused_variables)]
        let ephemeral = false;
        #[allow(unused_variables)]
        let network = None;
        #[allow(unused_variables)]
        let mounts = vec![];
        #[allow(unused_variables)]
        let privileged = false;
        #[allow(unused_variables)]
        let env = vec![];
        #[allow(unused_variables)]
        let name = None;
        #[allow(unused_variables)]
        let tty = false;
        #[allow(unused_variables)]
        let entrypoint = None;
        #[allow(unused_variables)]
        let user = None;
        #[allow(unused_variables)]
        let auto_remove = false;
        #[allow(unused_variables)]
        let priority = None;
        #[allow(unused_variables)]
        let urgency = None;
        #[allow(unused_variables)]
        let execution_context = None;
        // User overrides shadow the defaults above.
        $(#[allow(unused_variables)] let $field = $val;)*
        minibox_core::protocol::DaemonRequest::Run {
            image,
            tag,
            command,
            memory_limit_bytes,
            cpu_weight,
            ephemeral,
            network,
            mounts,
            privileged,
            env,
            name,
            tty,
            entrypoint,
            user,
            auto_remove,
            priority,
            urgency,
            execution_context,
        }
    }};
}
