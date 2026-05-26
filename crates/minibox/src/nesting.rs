//! Container nesting depth tracking and validation.
//!
//! Each container increments `MINIBOX_NEST_DEPTH` in its environment.
//! The daemon reads this to know its nesting level and enforce the
//! max depth limit (`MINIBOX_MAX_NEST_DEPTH`, default 4).

/// Default maximum nesting depth.
pub const DEFAULT_MAX_NEST_DEPTH: u32 = 4;

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
    pub fn new(depth: Option<u32>, max_depth: Option<u32>) -> Self {
        Self {
            depth: depth.unwrap_or(0),
            max_depth: max_depth.unwrap_or(DEFAULT_MAX_NEST_DEPTH),
        }
    }

    /// Read nesting context from the current process environment.
    pub fn from_env() -> Self {
        let depth = std::env::var("MINIBOX_NEST_DEPTH")
            .ok()
            .and_then(|v| v.parse().ok());
        let max = std::env::var("MINIBOX_MAX_NEST_DEPTH")
            .ok()
            .and_then(|v| v.parse().ok());
        Self::new(depth, max)
    }

    /// The depth value to set for a child container.
    pub fn child_depth(&self) -> u32 {
        self.depth + 1
    }

    /// Fail if current depth has reached or exceeded the limit.
    pub fn check_depth(&self) -> anyhow::Result<()> {
        if self.depth >= self.max_depth {
            anyhow::bail!(
                "nesting depth {} exceeds maximum (MINIBOX_MAX_NEST_DEPTH={})",
                self.depth,
                self.max_depth
            );
        }
        Ok(())
    }

    /// Environment variables to inject into child containers.
    pub fn child_env_vars(&self) -> Vec<String> {
        vec![
            format!("MINIBOX_NEST_DEPTH={}", self.child_depth()),
            format!("MINIBOX_MAX_NEST_DEPTH={}", self.max_depth),
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
    fn env_vars_for_child() {
        let ctx = NestingContext::new(Some(1), Some(8));
        let vars = ctx.child_env_vars();
        assert!(vars.contains(&"MINIBOX_NEST_DEPTH=2".to_string()));
        assert!(vars.contains(&"MINIBOX_MAX_NEST_DEPTH=8".to_string()));
    }
}
