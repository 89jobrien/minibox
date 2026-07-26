//! Conformance: verify smolbox re-exports are accessible and
//! the adapter types satisfy expected trait bounds.

#[test]
fn smolvm_runtime_is_accessible() {
    fn _assert_send<T: Send>() {}
    _assert_send::<smolbox::smolvm::SmolVmRuntime>();
}

#[test]
fn krun_runtime_is_accessible() {
    fn _assert_send<T: Send>() {}
    _assert_send::<smolbox::krun::KrunRuntime>();
}

#[test]
fn preflight_check_smolvm_returns_status() {
    let status = smolbox::preflight::check_smolvm();
    assert_eq!(status.found, status.path.is_some());
}
