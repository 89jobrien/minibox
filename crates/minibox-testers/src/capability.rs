//! Typed, self-describing conformance capabilities.
//!
//! A [`ConformanceCapability`] knows its name, whether it is supported by a
//! given backend, and the reason it is skipped when unsupported.

/// Why a conformance test was skipped.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SkipReason {
    /// The backend did not declare this capability in its `BackendCapabilitySet`.
    CapabilityNotDeclared { capability: &'static str },
    /// The backend declared the capability but the required external service
    /// (e.g. a local OCI registry) is not available in this environment.
    ExternalServiceUnavailable { service: &'static str },
    /// The test is platform-gated and the current platform does not support it.
    PlatformUnsupported { platform: &'static str },
}

impl SkipReason {
    /// Return a human-readable explanation.
    pub fn message(&self) -> String {
        match self {
            SkipReason::CapabilityNotDeclared { capability } => {
                format!("backend does not declare {capability} capability")
            }
            SkipReason::ExternalServiceUnavailable { service } => {
                format!("external service not available: {service}")
            }
            SkipReason::PlatformUnsupported { platform } => {
                format!("platform not supported: {platform}")
            }
        }
    }
}

/// A typed, self-describing conformance capability.
///
/// Implement this trait for each capability group. The conformance runner uses
/// it to determine whether to run or skip a test case and why.
pub trait ConformanceCapability: Send + Sync + 'static {
    /// Short identifier used in reports, e.g. `"Commit"`.
    fn name(&self) -> &'static str;

    /// Return `true` if the backend supports this capability and the test
    /// should run.
    fn is_supported(&self) -> bool;

    /// Return the reason this capability is skipped when `is_supported()` is
    /// `false`. Used in report rows.
    fn skip_reason(&self) -> SkipReason;
}

/// Check whether to skip a test and return the skip message if so.
///
/// Returns `Some(message)` if the test should be skipped, `None` if it should run.
pub fn should_skip(cap: &dyn ConformanceCapability) -> Option<String> {
    if cap.is_supported() {
        None
    } else {
        Some(cap.skip_reason().message())
    }
}

// ---------------------------------------------------------------------------
// Built-in capability descriptors
// ---------------------------------------------------------------------------

/// Capability: backend can commit a container FS diff to a new image.
pub struct CommitCapability {
    pub supported: bool,
}

impl ConformanceCapability for CommitCapability {
    fn name(&self) -> &'static str {
        "Commit"
    }
    fn is_supported(&self) -> bool {
        self.supported
    }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared {
            capability: "Commit",
        }
    }
}

/// Capability: backend can build an image from a Dockerfile context.
pub struct BuildCapability {
    pub supported: bool,
}

impl ConformanceCapability for BuildCapability {
    fn name(&self) -> &'static str {
        "BuildFromContext"
    }
    fn is_supported(&self) -> bool {
        self.supported
    }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared {
            capability: "BuildFromContext",
        }
    }
}

/// Capability: backend can push an image to a registry.
pub struct PushCapability {
    pub supported: bool,
}

impl ConformanceCapability for PushCapability {
    fn name(&self) -> &'static str {
        "PushToRegistry"
    }
    fn is_supported(&self) -> bool {
        self.supported
    }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared {
            capability: "PushToRegistry",
        }
    }
}

/// Capability: backend supports GC (image garbage collection).
pub struct GcCapability {
    pub supported: bool,
}

impl ConformanceCapability for GcCapability {
    fn name(&self) -> &'static str {
        "ImageGarbageCollection"
    }
    fn is_supported(&self) -> bool {
        self.supported
    }
    fn skip_reason(&self) -> SkipReason {
        SkipReason::CapabilityNotDeclared {
            capability: "ImageGarbageCollection",
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_skip_returns_none_when_supported() {
        let cap = CommitCapability { supported: true };
        assert!(should_skip(&cap).is_none());
    }

    #[test]
    fn should_skip_returns_message_when_unsupported() {
        let cap = CommitCapability { supported: false };
        let msg = should_skip(&cap).expect("should return skip message");
        assert!(
            msg.contains("Commit"),
            "message must mention capability name"
        );
    }

    #[test]
    fn skip_reason_message_is_human_readable() {
        let r = SkipReason::CapabilityNotDeclared { capability: "Exec" };
        assert!(r.message().contains("Exec"));
        let r2 = SkipReason::ExternalServiceUnavailable {
            service: "localhost:5000",
        };
        assert!(r2.message().contains("localhost:5000"));
    }
}
