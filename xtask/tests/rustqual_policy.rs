//! Regression checks for the daemon handler rustqual policy.

use std::fs;
use std::path::Path;

const HANDLER_POLICY: &str = "crates/minibox/src/daemon/handler/rustqual.toml";
const HANDLER_SOURCE: &str = "crates/minibox/src/daemon/handler";

#[test]
fn handler_rustqual_policy_is_centralized_and_bounded() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask must live directly under the workspace root");
    let policy_path = workspace.join(HANDLER_POLICY);
    let policy = fs::read_to_string(&policy_path)
        .unwrap_or_else(|error| panic!("read {}: {error}", policy_path.display()));
    let config: toml::Value = toml::from_str(&policy)
        .unwrap_or_else(|error| panic!("parse {}: {error}", policy_path.display()));

    let complexity = config
        .get("complexity")
        .and_then(toml::Value::as_table)
        .expect("handler policy must define [complexity]");
    assert_eq!(
        complexity
            .get("max_cognitive")
            .and_then(toml::Value::as_integer),
        Some(22)
    );
    assert_eq!(
        complexity
            .get("max_cyclomatic")
            .and_then(toml::Value::as_integer),
        Some(15)
    );
    assert_eq!(
        complexity
            .get("max_nesting_depth")
            .and_then(toml::Value::as_integer),
        Some(6)
    );
    assert_eq!(
        complexity
            .get("max_function_lines")
            .and_then(toml::Value::as_integer),
        Some(320)
    );

    assert_no_function_level_complexity_allows(&workspace.join(HANDLER_SOURCE));
}

fn assert_no_function_level_complexity_allows(directory: &Path) {
    for entry in fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("read {}: {error}", directory.display()))
    {
        let path = entry.expect("read handler directory entry").path();
        if path.is_dir() {
            assert_no_function_level_complexity_allows(&path);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            let source = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
            assert!(
                !source.contains("qual:allow(complexity)"),
                "{} must use the module-level rustqual policy instead of function-level complexity allowances",
                path.display()
            );
        }
    }
}
