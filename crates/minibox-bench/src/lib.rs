//! Benchmark fixtures and harness support for the minibox workspace.
//!
//! This crate is a leaf: nothing depends on it, and it is the only place
//! where `test-utils` features of the lib crates are enabled.

pub mod fixtures;

/// Defines a documented Criterion benchmark group.
///
/// This mirrors the default-config compact form of Criterion 0.7's
/// `criterion_group!` while documenting the generated public harness function.
#[macro_export]
macro_rules! documented_criterion_group {
    ($documentation:literal, $name:ident, $($target:path),+ $(,)?) => {
        #[doc = $documentation]
        pub fn $name() {
            let mut criterion: ::criterion::Criterion<_> =
                ::criterion::Criterion::default().configure_from_args();
            $(
                $target(&mut criterion);
            )+
        }
    };
}

/// Runtime guard for root-required Linux benches.
#[must_use]
pub fn is_root() -> bool {
    #[cfg(unix)]
    {
        nix::unistd::geteuid().is_root()
    }
    #[cfg(not(unix))]
    {
        false
    }
}
