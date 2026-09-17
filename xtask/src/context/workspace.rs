/// Stable identity used in portable test evidence.
///
/// Cargo package IDs embed checkout paths for path dependencies, so imported
/// evidence uses the workspace package name instead.
pub(super) fn stable_package_identity(package_name: &str) -> String {
    package_name.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn package_identity_is_checkout_independent() {
        assert_eq!(stable_package_identity("minibox-core"), "minibox-core");
    }
}
