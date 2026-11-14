//! Conformance tests for adapter Build capability.
//!
//! Verifies that adapters report `BuildFromContext` capability truthfully:
//! - Adapters with `BuildFromContext` = true must handle Build requests without error
//! - Adapters with `BuildFromContext` = false must return "not supported" error

#[cfg(test)]
mod tests {
    #[test]
    fn adapter_build_capability_is_truthful() {
        // Stub conformance test: verifies adapter Build capability matches behavior
        // TODO: Iterate over all adapters and verify capability truthfulness
        // TODO: Assert success when BuildFromContext = true
        // TODO: Assert NotSupported error when BuildFromContext = false
    }
}
