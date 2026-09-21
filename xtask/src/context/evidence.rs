//! Collects evidence data for the xtask context snapshot.

use super::model::{ExecutableTest, ProfileStatus, TestProfileResult, WorkspaceSnapshot};
use std::collections::BTreeSet;

pub(super) fn workspace_package_identities(workspace: &WorkspaceSnapshot) -> BTreeSet<String> {
    workspace
        .packages
        .iter()
        .map(|package| super::workspace::stable_package_identity(&package.name))
        .collect()
}

pub(super) fn validated_unique_tests(profiles: &[TestProfileResult]) -> Vec<ExecutableTest> {
    let mut tests = profiles
        .iter()
        .filter(|profile| profile.status == ProfileStatus::Validated)
        .flat_map(|profile| profile.executable_tests.iter().cloned())
        .collect::<Vec<_>>();
    tests.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
    tests.dedup_by(|left, right| left.stable_id == right.stable_id);
    tests
}
