//! [`PluginEnvironment`] backed by the real process environment.

use crate::plugin::PluginEnvironment;

/// Reads `CNI_*` and `MINIBOX_*` variables from the current process.
///
/// An empty value is reported as absent, matching what a runtime means by
/// "not supplied" and what [`CniRollout::from_environment`] expects for a
/// cleared unit-file override.
///
/// [`CniRollout::from_environment`]: crate::rollout::CniRollout::from_environment
#[derive(Debug, Clone, Copy, Default)]
pub struct ProcessEnvironment;

impl PluginEnvironment for ProcessEnvironment {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|value| !value.is_empty())
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_variable_is_absent() {
        assert!(
            ProcessEnvironment
                .get("MINIBOX_CNI_DEFINITELY_NOT_SET_9c1f4a")
                .is_none()
        );
    }
}
