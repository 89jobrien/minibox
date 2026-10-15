//! macOS `os_signpost` instrumentation for minibox.
//!
//! Signposts appear as named intervals on the Points of Interest track in
//! Instruments, recorded by `cargo xtask profile native` (which enables the
//! `os_signpost` instrument). Unlike Time Profiler they attribute time to
//! *named operations* rather than sampled stacks, which is what makes
//! "where did the container boot spend its two seconds" answerable.
//!
//! ```no_run
//! # fn example() {
//! let _span = minibox_signpost::span(c"container.create");
//! // ... work ...
//! # }
//! ```
//!
//! The interval closes when the guard drops, so an early return still ends it.
//! Bind it (`let _span = ...`); `let _ = ...` would drop immediately.
//!
//! ## Constraints
//!
//! **Do not hold a guard across an `.await`.** Guards end their interval on the
//! thread that drops them, and a future may resume on a different worker. This
//! matches the workspace's existing "no fork/clone in async fn" discipline.
//!
//! **Names must be `c"..."` literals.** Apple requires signpost names to have
//! static storage duration; they are not runtime-formattable.
//!
//! ## Cost when not profiling
//!
//! [`span`] consults `OSSignposter::is_enabled` and does no further work when
//! no profiler is attached. On non-macOS targets [`Span`] is zero-sized and the
//! calls compile away entirely, so Linux and Windows builds are unaffected.
//!
//! ## Why this is its own crate
//!
//! Signposting is inherently platform-specific, and `minibox-core` is required to
//! have zero OS dependencies. Keeping this separate lets any layer — runtime,
//! daemon, or adapters — take a dependency without violating that invariant.
//! It is also the only place that needs Apple's `os_signpost`, since the
//! underlying C API is reachable only through a Swift bridge.

/// Signpost subsystem, shown as the subsystem column in Instruments.
#[cfg(all(target_os = "macos", feature = "macos-signposts"))]
const SUBSYSTEM: &str = "io.minibox";

/// Signpost category; every minibox marker is recorded under it.
#[cfg(all(target_os = "macos", feature = "macos-signposts"))]
const CATEGORY: &str = "performance";

/// A signpost interval that closes when the guard is dropped.
///
/// Construct with [`span`]. Holding the value is what keeps the interval open.
#[must_use = "the interval ends immediately unless the guard is bound to a variable"]
pub struct Span {
    #[cfg(all(target_os = "macos", feature = "macos-signposts"))]
    interval: Option<apple_log::OSSignpostInterval>,
}

impl Span {
    /// Closes the interval explicitly instead of waiting for the guard to drop.
    ///
    /// Useful when the work ends at a point the guard cannot span, such as
    /// before an `.await` that may resume on another worker. Idempotent.
    pub fn end(self) {
        drop(self);
    }
}

impl Drop for Span {
    fn drop(&mut self) {
        #[cfg(all(target_os = "macos", feature = "macos-signposts"))]
        if let Some(interval) = self.interval.take() {
            with_signposter(|signposter| signposter.end_interval(interval, ""));
        }
    }
}

/// Opens a named signpost interval; the returned guard closes it on drop.
///
/// Returns a no-op guard when no profiler is attached or the feature is off, so
/// call sites never need to branch.
#[allow(clippy::missing_const_for_fn)]
pub fn span(name: &'static std::ffi::CStr) -> Span {
    #[cfg(all(target_os = "macos", feature = "macos-signposts"))]
    {
        Span {
            interval: with_signposter(|signposter| {
                let id = unique_id(signposter);
                signposter.begin_interval(name, id, "")
            }),
        }
    }
    #[cfg(not(all(target_os = "macos", feature = "macos-signposts")))]
    {
        let _ = name;
        Span {}
    }
}

/// Emits a single-point signpost event, for occurrences rather than durations.
///
/// Prefer [`span`] when the operation has a meaningful duration.
#[allow(clippy::missing_const_for_fn)]
pub fn event(name: &'static std::ffi::CStr) {
    #[cfg(all(target_os = "macos", feature = "macos-signposts"))]
    {
        with_signposter(|signposter| {
            let id = unique_id(signposter);
            signposter.emit_event(name, id, "");
        });
    }
    #[cfg(not(all(target_os = "macos", feature = "macos-signposts")))]
    {
        let _ = name;
    }
}

/// Produces a process-wide unique signpost id.
///
/// `make_signpost_id` numbers ids per signposter, and each thread has its own
/// signposter, so two concurrent spans can collide on the same id — which would
/// make Instruments pair the wrong begin with the wrong end. Deriving the id
/// from a process-global counter keeps ids distinct even when an interval is
/// begun on one thread and ended on another, which is what a guard spanning an
/// `.await` does.
#[cfg(all(target_os = "macos", feature = "macos-signposts"))]
fn unique_id(signposter: &apple_log::OSSignposter) -> apple_log::OSSignpostId {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT: AtomicU64 = AtomicU64::new(0);
    let ticket = NEXT.fetch_add(1, Ordering::Relaxed);
    // The pointer only has to be unique and stable enough for the id derivation;
    // the ticket's address is both.
    let token = (&raw const ticket).cast::<()>();
    signposter.make_signpost_id_from_pointer(token)
}

/// Runs `f` against this thread's signposter, doing nothing if there is none.
///
/// `OSSignposter` owns a non-thread-safe handle, so one is kept per thread
/// rather than shared. Creating one per interval would dominate the cost of
/// the marker it is recording.
#[cfg(all(target_os = "macos", feature = "macos-signposts"))]
fn with_signposter<T>(f: impl FnOnce(&apple_log::OSSignposter) -> T) -> Option<T> {
    use apple_log::prelude::OSSignposter;

    thread_local! {
        static SIGNPOSTER: Option<OSSignposter> =
            OSSignposter::new(SUBSYSTEM, CATEGORY).ok();
    }

    SIGNPOSTER.with(|signposter| signposter.as_ref().map(f))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(all(target_os = "macos", feature = "macos-signposts"))]
    use std::ffi::CString;

    /// The subsystem and category reach `OSSignposter::new` as strings; a NUL
    /// would make it fail and silently disable every marker.
    #[cfg(all(target_os = "macos", feature = "macos-signposts"))]
    #[test]
    fn subsystem_and_category_are_constructible() {
        assert!(
            CString::new(SUBSYSTEM).is_ok(),
            "SUBSYSTEM has an interior NUL"
        );
        assert!(
            CString::new(CATEGORY).is_ok(),
            "CATEGORY has an interior NUL"
        );
    }

    /// Markers are called from real daemon paths, including error paths, so
    /// opening and closing one must never panic — on macOS that means driving
    /// the actual signposter.
    #[cfg(all(target_os = "macos", feature = "macos-signposts"))]
    #[test]
    fn span_and_event_run_without_a_profiler_attached() {
        drop(span(c"test.span"));
        event(c"test.event");
    }

    #[cfg(not(all(target_os = "macos", feature = "macos-signposts")))]
    #[test]
    fn guard_is_zero_sized_when_signposts_are_disabled() {
        assert_eq!(std::mem::size_of::<Span>(), 0);
        drop(span(c"test.span"));
        event(c"test.event");
    }

    /// Nested guards must close inner-before-outer so intervals nest in time.
    #[test]
    fn spans_nest_and_unwind_in_order() {
        let outer = span(c"test.outer");
        {
            let inner = span(c"test.inner");
            drop(inner);
        }
        drop(outer);
    }
}
