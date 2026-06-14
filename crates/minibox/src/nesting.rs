//! Container nesting depth tracking and validation.
//!
//! Each container increments `MINIBOX_NEST_DEPTH` in its environment.
//! The daemon reads this to know its nesting level and enforce the
//! max depth limit (`MINIBOX_MAX_NEST_DEPTH`, default 4).

use std::sync::OnceLock;

/// Default maximum nesting depth.
pub const DEFAULT_MAX_NEST_DEPTH: u32 = 4;

/// Environment variable for current nesting depth.
pub const ENV_NEST_DEPTH: &str = "MINIBOX_NEST_DEPTH";
/// Environment variable for maximum nesting depth.
pub const ENV_MAX_NEST_DEPTH: &str = "MINIBOX_MAX_NEST_DEPTH";

static NESTED_OVERLAY_SUPPORT: OnceLock<bool> = OnceLock::new();

/// Check whether the kernel supports overlay-on-overlay mounts.
///
/// Performs an empirical probe: mounts a tmpfs, creates a base overlay,
/// then attempts a second overlay using the first as a lowerdir. The
/// result is cached for the process lifetime.
///
/// Returns `false` on non-Linux or if any mount fails.
pub fn supports_nested_overlay() -> bool {
    *NESTED_OVERLAY_SUPPORT.get_or_init(probe_nested_overlay)
}

#[cfg(target_os = "linux")]
fn probe_nested_overlay() -> bool {
    use nix::mount::{MntFlags, MsFlags, mount, umount2};
    use std::fs;

    let probe_dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(_) => return false,
    };
    let base = probe_dir.path();

    // Mount tmpfs as the base filesystem
    if mount(
        Some("tmpfs"),
        base,
        Some("tmpfs"),
        MsFlags::empty(),
        Some("size=4m"),
    )
    .is_err()
    {
        return false;
    }

    let result = (|| -> anyhow::Result<bool> {
        // First overlay: lower1 -> merged1
        for d in &[
            "lower1", "upper1", "work1", "merged1", "upper2", "work2", "merged2",
        ] {
            fs::create_dir_all(base.join(d))?;
        }
        fs::write(base.join("lower1/probe.txt"), "probe")?;

        let opts1 = format!(
            "lowerdir={lower},upperdir={upper},workdir={work}",
            lower = base.join("lower1").display(),
            upper = base.join("upper1").display(),
            work = base.join("work1").display(),
        );
        mount(
            Some("overlay"),
            &base.join("merged1"),
            Some("overlay"),
            MsFlags::empty(),
            Some(opts1.as_str()),
        )?;

        // Second overlay: use merged1 as lowerdir
        let opts2 = format!(
            "lowerdir={lower},upperdir={upper},workdir={work}",
            lower = base.join("merged1").display(),
            upper = base.join("upper2").display(),
            work = base.join("work2").display(),
        );
        let nested_ok = mount(
            Some("overlay"),
            &base.join("merged2"),
            Some("overlay"),
            MsFlags::empty(),
            Some(opts2.as_str()),
        )
        .is_ok();

        // Cleanup
        let _ = umount2(&base.join("merged2"), MntFlags::MNT_DETACH);
        let _ = umount2(&base.join("merged1"), MntFlags::MNT_DETACH);

        Ok(nested_ok)
    })();

    let _ = umount2(base, MntFlags::MNT_DETACH);
    result.unwrap_or(false)
}

#[cfg(not(target_os = "linux"))]
const fn probe_nested_overlay() -> bool {
    false
}

/// Nesting metadata passed through the container init path.
#[derive(Debug, Clone)]
pub struct NestingContext {
    /// Current depth (0 = host, 1 = first container, 2 = nested, ...).
    pub depth: u32,
    /// Maximum allowed depth. Container init fails if depth >= max.
    pub max_depth: u32,
}

impl NestingContext {
    /// Build from optional env values. `None` depth means host (0).
    #[must_use]
    pub fn new(depth: Option<u32>, max_depth: Option<u32>) -> Self {
        Self {
            depth: depth.unwrap_or(0),
            max_depth: max_depth.unwrap_or(DEFAULT_MAX_NEST_DEPTH),
        }
    }

    /// Read nesting context from the current process environment.
    #[must_use]
    pub fn from_env() -> Self {
        let depth = std::env::var(ENV_NEST_DEPTH)
            .ok()
            .and_then(|v| v.parse().ok());
        let max = std::env::var(ENV_MAX_NEST_DEPTH)
            .ok()
            .and_then(|v| v.parse().ok());
        Self::new(depth, max)
    }

    /// The depth value to set for a child container.
    #[must_use]
    pub const fn child_depth(&self) -> u32 {
        self.depth.saturating_add(1)
    }

    /// Fail if current depth has reached or exceeded the limit.
    pub fn check_depth(&self) -> anyhow::Result<()> {
        if self.depth >= self.max_depth {
            anyhow::bail!(
                "nesting depth {} exceeds maximum ({ENV_MAX_NEST_DEPTH}={})",
                self.depth,
                self.max_depth
            );
        }
        Ok(())
    }

    /// Environment variables to inject into child containers.
    #[must_use]
    pub fn child_env_vars(&self) -> Vec<String> {
        vec![
            format!("{ENV_NEST_DEPTH}={}", self.child_depth()),
            format!("{ENV_MAX_NEST_DEPTH}={}", self.max_depth),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_env_absent_is_depth_zero() {
        let ctx = NestingContext::new(None, None);
        assert_eq!(ctx.depth, 0);
        assert_eq!(ctx.max_depth, DEFAULT_MAX_NEST_DEPTH);
    }

    #[test]
    fn from_env_increments_depth() {
        let ctx = NestingContext::new(Some(2), None);
        assert_eq!(ctx.depth, 2);
    }

    #[test]
    fn child_depth_increments() {
        let ctx = NestingContext::new(Some(1), None);
        assert_eq!(ctx.child_depth(), 2);
    }

    #[test]
    fn check_depth_ok_within_limit() {
        let ctx = NestingContext::new(Some(3), Some(4));
        assert!(ctx.check_depth().is_ok());
    }

    #[test]
    fn check_depth_fails_at_limit() {
        let ctx = NestingContext::new(Some(4), Some(4));
        let err = ctx.check_depth().unwrap_err();
        assert!(err.to_string().contains("nesting depth"));
    }

    #[test]
    fn check_depth_fails_over_limit() {
        let ctx = NestingContext::new(Some(5), Some(4));
        assert!(ctx.check_depth().is_err());
    }

    #[test]
    fn custom_max_depth() {
        let ctx = NestingContext::new(Some(1), Some(2));
        assert_eq!(ctx.max_depth, 2);
    }

    #[test]
    fn probe_result_is_cached() {
        let a = supports_nested_overlay();
        let b = supports_nested_overlay();
        assert_eq!(a, b);
    }

    #[test]
    fn env_vars_for_child() {
        let ctx = NestingContext::new(Some(1), Some(8));
        let vars = ctx.child_env_vars();
        assert!(vars.contains(&"MINIBOX_NEST_DEPTH=2".to_string()));
        assert!(vars.contains(&"MINIBOX_MAX_NEST_DEPTH=8".to_string()));
    }

    #[test]
    fn child_depth_saturates_at_u32_max() {
        let ctx = NestingContext::new(Some(u32::MAX), Some(u32::MAX));
        assert_eq!(ctx.child_depth(), u32::MAX);
    }

    #[test]
    fn check_depth_zero_max_zero_fails() {
        let ctx = NestingContext::new(Some(0), Some(0));
        assert!(ctx.check_depth().is_err());
    }

    #[test]
    fn check_depth_zero_max_one_ok() {
        let ctx = NestingContext::new(Some(0), Some(1));
        assert!(ctx.check_depth().is_ok());
    }
}
