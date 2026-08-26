//! Compatibility facade for the pure [`minibox_domain`] inner ring.
//!
//! New consumers may depend on `minibox-domain` directly. Existing consumers
//! can continue using `minibox_core::domain::*`; both paths resolve to the same
//! types and traits rather than parallel definitions.

pub use minibox_domain::*;
