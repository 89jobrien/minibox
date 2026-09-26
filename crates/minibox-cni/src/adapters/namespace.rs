//! [`NamespaceAttachment`] for hosts where minibox does not yet ship a
//! netlink backend.
//!
//! # Status
//!
//! `ADD` and `CHECK` report a structured CNI error rather than silently
//! reporting a result for networking that was never created. `DEL` is a
//! no-op, which is spec-correct: the spec requires teardown to be
//! idempotent, and nothing can exist to tear down while `ADD` always fails.
//!
//! The remaining work is a netlink `NamespaceAttachment` implementation;
//! the protocol, error mapping, and rollout around it are complete and
//! tested.

use crate::error::CniError;
use crate::plugin::{AttachmentRequest, NamespaceAttachment};
use crate::result::CniResult;
use crate::version::ERR_INTERNAL;

/// Explanation attached to every refusal, so an operator reading the
/// runtime's logs learns why the plugin is not doing namespace work yet.
pub const DEFERRED_DETAIL: &str = "minibox ships no netlink backend for namespace attachment; \
the CNI protocol handshake, spec-version validation, and rollout preflight are complete, but \
ADD/CHECK cannot move a veth into the container namespace until that adapter lands";

/// The adapter the packaged `minibox-bridge` binary uses today.
#[derive(Debug, Clone)]
pub struct UnimplementedNamespaceAttachment {
    /// The plugin's `type`, as written in the `.conflist`, for error reports.
    plugin_name: String,
}

impl UnimplementedNamespaceAttachment {
    /// Build the adapter for a plugin named `plugin_name`.
    #[must_use]
    pub fn new(plugin_name: impl Into<String>) -> Self {
        Self {
            plugin_name: plugin_name.into(),
        }
    }

    fn deferred(&self) -> CniError {
        CniError::PluginError {
            plugin: self.plugin_name.clone(),
            code: Some(ERR_INTERNAL),
            msg: "namespace attachment is not implemented by this build".to_string(),
            details: Some(DEFERRED_DETAIL.to_string()),
        }
    }
}

impl NamespaceAttachment for UnimplementedNamespaceAttachment {
    fn attach(&self, _request: &AttachmentRequest) -> Result<CniResult, CniError> {
        Err(self.deferred())
    }

    fn check(&self, _request: &AttachmentRequest) -> Result<CniResult, CniError> {
        Err(self.deferred())
    }

    fn detach(&self, _request: &AttachmentRequest) -> Result<(), CniError> {
        // Nothing can have been created while attach() always fails, so
        // teardown is a genuine no-op rather than a silent failure.
        tracing::warn!(plugin = %self.plugin_name, "cni: DEL is a no-op; namespace attachment is not implemented");
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use crate::plugin::error_payload;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    fn request() -> AttachmentRequest {
        AttachmentRequest {
            network_name: "minibox0".to_string(),
            container_id: "container-1".to_string(),
            ifname: "eth0".to_string(),
            netns: "/proc/1/ns/net".to_string(),
            bridge: Some("minibox0".to_string()),
            cni_path: vec![PathBuf::from("/opt/cni/bin")],
            cni_args: BTreeMap::new(),
        }
    }

    #[test]
    fn attach_refuses_loudly_rather_than_faking_a_result() {
        let adapter = UnimplementedNamespaceAttachment::new("minibox-bridge");
        let result = adapter.attach(&request());
        match result {
            Err(CniError::PluginError {
                plugin,
                code,
                details,
                ..
            }) => {
                assert_eq!(plugin, "minibox-bridge");
                assert_eq!(code, Some(ERR_INTERNAL));
                assert!(details.expect("details").contains("netlink"));
            }
            other => panic!("expected PluginError, got {other:?}"),
        }
    }

    #[test]
    fn check_refuses_loudly_rather_than_faking_a_result() {
        let adapter = UnimplementedNamespaceAttachment::new("minibox-bridge");
        assert!(adapter.check(&request()).is_err());
    }

    #[test]
    fn detach_is_idempotent() {
        let adapter = UnimplementedNamespaceAttachment::new("minibox-bridge");
        assert!(adapter.detach(&request()).is_ok());
        assert!(adapter.detach(&request()).is_ok());
    }

    #[test]
    fn the_refusal_is_a_decodable_cni_error_payload() {
        let adapter = UnimplementedNamespaceAttachment::new("minibox-bridge");
        let error = adapter.attach(&request()).expect_err("attach must refuse");
        let payload = error_payload(&error);
        assert_eq!(payload.code, ERR_INTERNAL);
        assert!(!payload.msg.is_empty());
        assert!(payload.details.expect("details").contains("netlink"));
    }
}
