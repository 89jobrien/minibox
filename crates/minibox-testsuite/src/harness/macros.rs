//! `ConformanceTestEntry` and inventory collection for the
//! `conformance_test!` macro.

use super::traits::ConformanceTest;

/// Wrapper submitted to `inventory` by each `conformance_test!`
/// invocation.
pub struct ConformanceTestEntry {
    /// Factory function producing a boxed conformance test.
    pub make: fn() -> Box<dyn ConformanceTest>,
}

inventory::collect!(ConformanceTestEntry);
