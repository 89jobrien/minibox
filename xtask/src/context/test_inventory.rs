use super::collect::{CommandRunner, CommandSpec, RepositoryReader, redact_diagnostic};
use super::manifest::ValidationProfile;
use super::model::{
    EvidenceId, ExecutableTest, ProfileStatus, RepositoryPath, SourceTestDeclaration,
    TestDeclarationKind, TestProfileResult, WorkspaceSnapshot,
};
use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use syn::visit::Visit;

pub(super) fn collect_source_tests(
    repository: &impl RepositoryReader,
    root: &Path,
    workspace: &WorkspaceSnapshot,
) -> Result<Vec<SourceTestDeclaration>> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize workspace root {}", root.display()))?;
    let tracked_files = repository.tracked_files(&root)?;
    let mut declarations = Vec::new();

    for path in tracked_files {
        if Path::new(path.as_str())
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("rs")
        {
            continue;
        }
        let Some(package_index) = owning_package_index(&path, workspace) else {
            continue;
        };
        let package = &workspace.packages[package_index];
        let source = repository.read_utf8(&root.join(path.as_str()))?;
        let syntax = syn::parse_file(&source)
            .with_context(|| format!("parse Rust source {}", path.as_str()))?;
        let mut visitor = DeclarationVisitor {
            package_id: package.package_id.clone(),
            path,
            module_path: Vec::new(),
            inherited_cfg: Vec::new(),
            declarations: Vec::new(),
            error: None,
        };
        visitor.visit_file(&syntax);
        if let Some(error) = visitor.error {
            return Err(error);
        }
        declarations.extend(visitor.declarations);
    }

    declarations.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
    Ok(declarations)
}

fn owning_package_index(path: &RepositoryPath, workspace: &WorkspaceSnapshot) -> Option<usize> {
    let path = Path::new(path.as_str());
    workspace
        .packages
        .iter()
        .enumerate()
        .filter_map(|(index, package)| {
            let root = Path::new(package.manifest_path.as_str()).parent()?;
            path.starts_with(root)
                .then_some((index, root.components().count()))
        })
        .max_by_key(|(_, depth)| *depth)
        .map(|(index, _)| index)
}

struct DeclarationVisitor {
    package_id: String,
    path: RepositoryPath,
    module_path: Vec<String>,
    inherited_cfg: Vec<String>,
    declarations: Vec<SourceTestDeclaration>,
    error: Option<anyhow::Error>,
}

impl DeclarationVisitor {
    fn record(&mut self, name: String, kind: TestDeclarationKind, attributes: &[syn::Attribute]) {
        let mut cfg_predicates = self.inherited_cfg.clone();
        cfg_predicates.extend(cfg_predicates_from(attributes));
        cfg_predicates.sort();
        cfg_predicates.dedup();
        let module = if self.module_path.is_empty() {
            "<root>".to_string()
        } else {
            self.module_path.join("::")
        };
        let kind_name = match kind {
            TestDeclarationKind::Function => "function",
            TestDeclarationKind::MacroInvocation => "macro_invocation",
        };
        self.declarations.push(SourceTestDeclaration {
            stable_id: format!(
                "{}::{}::{module}::{name}::{kind_name}",
                self.package_id,
                self.path.as_str()
            ),
            package_id: self.package_id.clone(),
            path: self.path.clone(),
            module_path: self.module_path.clone(),
            name,
            cfg_predicates,
            kind,
        });
    }
}

impl<'ast> Visit<'ast> for DeclarationVisitor {
    fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
        let module_depth = self.module_path.len();
        let cfg_depth = self.inherited_cfg.len();
        self.module_path.push(item.ident.to_string());
        self.inherited_cfg.extend(cfg_predicates_from(&item.attrs));
        if let Some((_, items)) = &item.content {
            for item in items {
                self.visit_item(item);
            }
        }
        self.module_path.truncate(module_depth);
        self.inherited_cfg.truncate(cfg_depth);
    }

    fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
        if item.attrs.iter().any(is_test_attribute) {
            self.record(
                item.sig.ident.to_string(),
                TestDeclarationKind::Function,
                &item.attrs,
            );
        }
    }

    fn visit_item_macro(&mut self, item: &'ast syn::ItemMacro) {
        if item
            .mac
            .path
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "conformance_test")
        {
            match conformance_test_name(item) {
                Ok(name) => self.record(name, TestDeclarationKind::MacroInvocation, &item.attrs),
                Err(error) => self.error = Some(error),
            }
        }
    }
}

fn is_test_attribute(attribute: &syn::Attribute) -> bool {
    attribute
        .path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "test")
}

fn cfg_predicates_from(attributes: &[syn::Attribute]) -> Vec<String> {
    attributes
        .iter()
        .filter_map(|attribute| {
            let name = attribute.path().segments.last()?.ident.to_string();
            if name != "cfg" && name != "cfg_attr" {
                return None;
            }
            let syn::Meta::List(list) = &attribute.meta else {
                return None;
            };
            let tokens = list
                .tokens
                .to_string()
                .split_whitespace()
                .collect::<String>();
            Some(format!("{name}({tokens})"))
        })
        .collect()
}

fn conformance_test_name(item: &syn::ItemMacro) -> Result<String> {
    let mut tokens = item.mac.tokens.clone().into_iter();
    let label = tokens
        .next()
        .context("conformance_test! is missing its name label")?;
    if label.to_string() != "name" {
        bail!("conformance_test! must begin with a name field");
    }
    let separator = tokens
        .next()
        .context("conformance_test! name is missing a colon")?;
    if separator.to_string() != ":" {
        bail!("conformance_test! name must be followed by a colon");
    }
    let literal = tokens
        .next()
        .context("conformance_test! is missing its name value")?;
    let name = syn::parse_str::<syn::LitStr>(&literal.to_string())
        .context("conformance_test! name must be a string literal")?;
    Ok(name.value())
}

pub(super) fn collect_test_profile(
    runner: &impl CommandRunner,
    root: &Path,
    profile: &ValidationProfile,
) -> TestProfileResult {
    let mut features = profile.features.clone();
    features.sort();
    features.dedup();
    let mut args = vec![
        "nextest".to_string(),
        "list".to_string(),
        "--workspace".to_string(),
        "--message-format".to_string(),
        "json".to_string(),
        "--target".to_string(),
        profile.target.clone(),
    ];
    if !features.is_empty() {
        args.extend(["--features".to_string(), features.join(",")]);
    }
    if profile.no_default_features {
        args.push("--no-default-features".to_string());
    }
    if profile.all_targets {
        args.push("--all-targets".to_string());
    }

    let command = CommandSpec {
        program: "cargo".to_string(),
        args,
        current_dir: root.to_path_buf(),
    };
    let evidence_ids = vec![EvidenceId::from("nextest:list")];
    let output = match runner.run(&command) {
        Ok(output) => output,
        Err(error) => {
            return TestProfileResult {
                profile_id: profile.id.clone(),
                target: profile.target.clone(),
                features,
                status: ProfileStatus::Failed,
                executable_tests: Vec::new(),
                unavailable_reason: Some(redact_diagnostic(root, &format!("{error:#}"))),
                evidence_ids,
            };
        }
    };

    if output.exit_code == Some(0) {
        return match std::str::from_utf8(&output.stdout)
            .context("nextest JSON output is not UTF-8")
            .and_then(parse_nextest_listing)
        {
            Ok(executable_tests) => TestProfileResult {
                profile_id: profile.id.clone(),
                target: profile.target.clone(),
                features,
                status: ProfileStatus::Validated,
                executable_tests,
                unavailable_reason: None,
                evidence_ids,
            },
            Err(error) => TestProfileResult {
                profile_id: profile.id.clone(),
                target: profile.target.clone(),
                features,
                status: ProfileStatus::Failed,
                executable_tests: Vec::new(),
                unavailable_reason: Some(redact_diagnostic(root, &format!("{error:#}"))),
                evidence_ids,
            },
        };
    }

    let diagnostic = String::from_utf8_lossy(&output.stderr);
    let status = if is_missing_target_or_toolchain(&diagnostic) {
        ProfileStatus::Unavailable
    } else {
        ProfileStatus::Failed
    };
    TestProfileResult {
        profile_id: profile.id.clone(),
        target: profile.target.clone(),
        features,
        status,
        executable_tests: Vec::new(),
        unavailable_reason: Some(redact_diagnostic(root, &diagnostic)),
        evidence_ids,
    }
}

fn is_missing_target_or_toolchain(diagnostic: &str) -> bool {
    let diagnostic = diagnostic.to_ascii_lowercase();
    diagnostic.contains("target may not be installed")
        || diagnostic.contains("could not find specification for target")
        || diagnostic.contains("can't find crate for `std`")
        || diagnostic.contains("can't find crate for 'std'")
        || (diagnostic.contains("toolchain")
            && (diagnostic.contains("not installed") || diagnostic.contains("not found")))
        || (diagnostic.contains("component") && diagnostic.contains("unavailable for download"))
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct NextestListing {
    #[serde(rename = "rust-build-meta")]
    _rust_build_meta: BTreeMap<String, serde_json::Value>,
    test_count: usize,
    rust_suites: BTreeMap<String, NextestSuite>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct NextestSuite {
    package_name: String,
    binary_id: String,
    binary_name: String,
    #[serde(rename = "package-id")]
    _package_id: String,
    kind: String,
    binary_path: String,
    build_platform: String,
    cwd: String,
    status: String,
    testcases: BTreeMap<String, NextestTestCase>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct NextestTestCase {
    kind: String,
    ignored: bool,
    filter_match: NextestFilterMatch,
}

#[derive(Deserialize)]
struct NextestFilterMatch {
    status: String,
}

pub(super) fn parse_nextest_listing(json: &str) -> Result<Vec<ExecutableTest>> {
    let listing: NextestListing =
        serde_json::from_str(json).context("parse nextest full JSON listing")?;
    let mut tests = Vec::with_capacity(listing.test_count);
    let mut stable_ids = BTreeSet::new();

    for (suite_key, suite) in listing.rust_suites {
        if suite_key != suite.binary_id {
            bail!(
                "nextest suite key {suite_key:?} does not match binary id {:?}",
                suite.binary_id
            );
        }
        if [
            suite.package_name.as_str(),
            suite.binary_name.as_str(),
            suite.kind.as_str(),
            suite.binary_path.as_str(),
            suite.build_platform.as_str(),
            suite.cwd.as_str(),
            suite.status.as_str(),
        ]
        .contains(&"")
        {
            bail!("nextest suite {suite_key:?} contains an empty required field");
        }
        for (test_name, testcase) in suite.testcases {
            if testcase.kind != "test" || testcase.filter_match.status.is_empty() {
                bail!(
                    "nextest testcase {test_name:?} in {suite_key:?} has incompatible schema values"
                );
            }
            let package_id = super::workspace::stable_package_identity(&suite.package_name);
            let stable_id = format!("{package_id}::{}::{test_name}", suite.binary_id);
            if !stable_ids.insert(stable_id.clone()) {
                bail!("duplicate nextest test identity: {stable_id}");
            }
            tests.push(ExecutableTest {
                stable_id,
                package_id,
                binary_id: suite.binary_id.clone(),
                test_name,
                ignored: testcase.ignored,
            });
        }
    }
    if tests.len() != listing.test_count {
        bail!(
            "nextest test-count {} does not match {} listed testcases",
            listing.test_count,
            tests.len()
        );
    }
    tests.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
    Ok(tests)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::collect::{
        CommandOutput, CommandRunner, CommandSpec, SystemCommandRunner, SystemRepositoryReader,
        collect_workspace,
    };
    use crate::context::manifest::ValidationProfile;
    use crate::context::model::{ProfileStatus, TestDeclarationKind};
    use std::process::Command;

    #[test]
    fn source_inventory_retains_cfg_and_macro_declarations() {
        let temp = tempfile::tempdir().expect("temporary workspace should be created");
        let root = temp.path();
        std::fs::create_dir_all(root.join("sample/src"))
            .expect("sample source directory should be created");
        std::fs::write(
            root.join("Cargo.toml"),
            r#"[workspace]
resolver = "3"
members = ["sample"]
"#,
        )
        .expect("workspace manifest should be written");
        std::fs::write(
            root.join("sample/Cargo.toml"),
            r#"[package]
name = "inventory-sample"
version = "0.1.0"
edition = "2024"
"#,
        )
        .expect("package manifest should be written");
        std::fs::write(
            root.join("sample/src/lib.rs"),
            r#"#[cfg(unix)]
mod outer {
    #[cfg(feature = "alpha")]
    #[test]
    fn plain() {}

    mod inner {
        #[tokio::test]
        async fn async_case() {}
    }
}

#[cfg_attr(target_os = "linux", cfg(feature = "linux-only"))]
#[test]
fn cfg_attr_case() {}

crate::conformance_test! {
    name: "macro_case",
    adapter: "sample",
    capability: Run,
    category: Unit,
    |ctx| { ctx.result() }
}
"#,
        )
        .expect("Rust inventory fixture should be written");
        for args in [vec!["init", "--quiet"], vec!["add", "--all"]] {
            let status = Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .expect("git fixture command should run");
            assert!(status.success(), "git fixture command should succeed");
        }

        let repository = SystemRepositoryReader;
        let workspace = collect_workspace(&SystemCommandRunner, &repository, root)
            .expect("workspace metadata should be collected");
        let package_id = workspace
            .packages
            .iter()
            .find(|package| package.name == "inventory-sample")
            .expect("fixture package should be present")
            .package_id
            .clone();
        let declarations = collect_source_tests(&repository, root, &workspace)
            .expect("source declarations should be collected");
        assert_eq!(declarations.len(), 4);

        let declaration = |name: &str| {
            declarations
                .iter()
                .find(|declaration| declaration.name == name)
                .expect("named declaration should be present")
        };
        let plain = declaration("plain");
        assert_eq!(plain.package_id, package_id);
        assert_eq!(plain.path.as_str(), "sample/src/lib.rs");
        assert_eq!(plain.module_path, ["outer"]);
        assert_eq!(
            plain.cfg_predicates,
            ["cfg(feature=\"alpha\")", "cfg(unix)"]
        );
        assert_eq!(plain.kind, TestDeclarationKind::Function);
        assert_eq!(
            plain.stable_id,
            format!("{}::sample/src/lib.rs::outer::plain::function", package_id)
        );

        let async_case = declaration("async_case");
        assert_eq!(async_case.module_path, ["outer", "inner"]);
        assert_eq!(async_case.cfg_predicates, ["cfg(unix)"]);
        assert_eq!(async_case.kind, TestDeclarationKind::Function);

        let cfg_attr = declaration("cfg_attr_case");
        assert!(cfg_attr.module_path.is_empty());
        assert_eq!(
            cfg_attr.cfg_predicates,
            ["cfg_attr(target_os=\"linux\",cfg(feature=\"linux-only\"))"]
        );

        let macro_case = declaration("macro_case");
        assert!(macro_case.module_path.is_empty());
        assert!(macro_case.cfg_predicates.is_empty());
        assert_eq!(macro_case.kind, TestDeclarationKind::MacroInvocation);
        assert_eq!(
            macro_case.stable_id,
            format!(
                "{}::sample/src/lib.rs::<root>::macro_case::macro_invocation",
                package_id
            )
        );

        assert!(
            declarations
                .windows(2)
                .all(|pair| pair[0].stable_id < pair[1].stable_id)
        );
        assert!(
            declarations
                .iter()
                .all(|declaration| declaration.path.as_str() == "sample/src/lib.rs")
        );
    }

    #[test]
    fn source_inventory_rejects_malformed_rust_and_conformance_macros() {
        let temp = tempfile::tempdir().expect("temporary workspace should be created");
        let root = temp.path();
        std::fs::create_dir_all(root.join("sample/src"))
            .expect("sample source directory should be created");
        std::fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nresolver = \"3\"\nmembers = [\"sample\"]\n",
        )
        .expect("workspace manifest should be written");
        std::fs::write(
            root.join("sample/Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .expect("package manifest should be written");
        std::fs::write(root.join("sample/src/lib.rs"), "pub fn initial() {}\n")
            .expect("initial source should be written");
        for args in [vec!["init", "--quiet"], vec!["add", "--all"]] {
            let status = Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .expect("git fixture command should run");
            assert!(status.success());
        }
        let repository = SystemRepositoryReader;
        let workspace = collect_workspace(&SystemCommandRunner, &repository, root)
            .expect("workspace should collect");
        std::fs::write(root.join("sample/src/lib.rs"), "fn broken( {")
            .expect("malformed source should be written");
        assert!(collect_source_tests(&repository, root, &workspace).is_err());
        std::fs::write(
            root.join("sample/src/lib.rs"),
            "crate::conformance_test! { adapter: \"native\" }\n",
        )
        .expect("malformed macro should be written");
        assert!(collect_source_tests(&repository, root, &workspace).is_err());
    }
    #[test]
    fn nextest_json_preserves_binary_and_test_identity() {
        let fixture = r#"{
          "rust-build-meta": {},
          "test-count": 4,
          "rust-suites": {
            "sample::lib": {
              "package-name": "sample",
              "binary-id": "sample::lib",
              "binary-name": "sample",
              "package-id": "path+file:///workspace/sample#0.1.0",
              "kind": "lib",
              "binary-path": "/workspace/target/sample-lib",
              "build-platform": "target",
              "cwd": "/workspace/sample",
              "status": "listed",
              "testcases": {
                "repeated_name": {
                  "kind": "test",
                  "ignored": false,
                  "filter-match": { "status": "matches" }
                },
                "ignored_case": {
                  "kind": "test",
                  "ignored": true,
                  "filter-match": { "status": "matches" }
                }
              }
            },
            "sample::bin/tool": {
              "package-name": "sample",
              "binary-id": "sample::bin/tool",
              "binary-name": "tool",
              "package-id": "path+file:///workspace/sample#0.1.0",
              "kind": "bin",
              "binary-path": "/workspace/target/tool",
              "build-platform": "target",
              "cwd": "/workspace/sample",
              "status": "listed",
              "testcases": {
                "repeated_name": {
                  "kind": "test",
                  "ignored": false,
                  "filter-match": { "status": "matches" }
                }
              }
            },
            "sample::test/integration": {
              "package-name": "sample",
              "binary-id": "sample::test/integration",
              "binary-name": "integration",
              "package-id": "path+file:///workspace/sample#0.1.0",
              "kind": "test",
              "binary-path": "/workspace/target/integration",
              "build-platform": "target",
              "cwd": "/workspace/sample",
              "status": "listed",
              "testcases": {
                "test result: ok. 1 passed": {
                  "kind": "test",
                  "ignored": false,
                  "filter-match": { "status": "matches" }
                }
              }
            },
            "sample::test/empty": {
              "package-name": "sample",
              "binary-id": "sample::test/empty",
              "binary-name": "empty",
              "package-id": "path+file:///workspace/sample#0.1.0",
              "kind": "test",
              "binary-path": "/workspace/target/empty",
              "build-platform": "target",
              "cwd": "/workspace/sample",
              "status": "listed",
              "testcases": {}
            }
          }
        }"#;

        let tests = parse_nextest_listing(fixture).expect("nextest JSON fixture should parse");
        assert_eq!(tests.len(), 4);
        assert!(
            tests
                .windows(2)
                .all(|pair| pair[0].stable_id < pair[1].stable_id)
        );

        let repeated = tests
            .iter()
            .filter(|test| test.test_name == "repeated_name")
            .collect::<Vec<_>>();
        assert_eq!(repeated.len(), 2);
        assert_eq!(repeated[0].package_id, "sample");
        assert_ne!(repeated[0].binary_id, repeated[1].binary_id);
        assert_ne!(repeated[0].stable_id, repeated[1].stable_id);
        for test in repeated {
            assert_eq!(
                test.stable_id,
                format!(
                    "{}::{}::{}",
                    test.package_id, test.binary_id, test.test_name
                )
            );
        }

        let ignored = tests
            .iter()
            .find(|test| test.test_name == "ignored_case")
            .expect("ignored test should be retained");
        assert!(ignored.ignored);
        assert!(
            tests
                .iter()
                .any(|test| test.test_name == "test result: ok. 1 passed")
        );
        assert!(!tests.iter().any(|test| test.binary_id.ends_with("/empty")));

        assert!(parse_nextest_listing("test result: ok. 4 passed").is_err());
        assert!(parse_nextest_listing("{}").is_err());
    }
    struct FakeRunner {
        output: CommandOutput,
        seen: std::cell::RefCell<Vec<CommandSpec>>,
    }

    impl FakeRunner {
        fn new(exit_code: i32, stdout: &str, stderr: &str) -> Self {
            Self {
                output: CommandOutput {
                    exit_code: Some(exit_code),
                    stdout: stdout.as_bytes().to_vec(),
                    stderr: stderr.as_bytes().to_vec(),
                },
                seen: std::cell::RefCell::new(Vec::new()),
            }
        }
    }

    impl CommandRunner for FakeRunner {
        fn run(&self, command: &CommandSpec) -> Result<CommandOutput> {
            self.seen.borrow_mut().push(command.clone());
            Ok(self.output.clone())
        }
    }

    fn validation_profile() -> ValidationProfile {
        ValidationProfile {
            id: "linux-features".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            features: vec!["zeta".to_string(), "alpha".to_string()],
            no_default_features: true,
            all_targets: true,
            required_in_ci: true,
        }
    }

    #[test]
    fn profile_collection_distinguishes_failure_and_unavailable() {
        let temp = tempfile::tempdir().expect("temporary profile root should be created");
        let valid_json = r#"{
          "rust-build-meta": {},
          "test-count": 1,
          "rust-suites": {
            "sample::lib": {
              "package-name": "sample",
              "binary-id": "sample::lib",
              "binary-name": "sample",
              "package-id": "path+file:///workspace/sample#0.1.0",
              "kind": "lib",
              "binary-path": "/workspace/target/sample",
              "build-platform": "target",
              "cwd": "/workspace/sample",
              "status": "listed",
              "testcases": {
                "works": {
                  "kind": "test",
                  "ignored": false,
                  "filter-match": { "status": "matches" }
                }
              }
            }
          }
        }"#;
        let runner = FakeRunner::new(0, valid_json, "");
        let validated = collect_test_profile(&runner, temp.path(), &validation_profile());
        assert_eq!(validated.status, ProfileStatus::Validated);
        assert_eq!(validated.executable_tests.len(), 1);
        assert_eq!(validated.features, ["alpha", "zeta"]);
        assert!(validated.unavailable_reason.is_none());
        assert_eq!(
            runner.seen.borrow()[0].args,
            [
                "nextest",
                "list",
                "--workspace",
                "--message-format",
                "json",
                "--target",
                "x86_64-unknown-linux-gnu",
                "--features",
                "alpha,zeta",
                "--no-default-features",
                "--all-targets",
            ]
        );

        let missing_message = format!(
            "error: target may not be installed; searched {}",
            temp.path().display()
        );
        let unavailable = collect_test_profile(
            &FakeRunner::new(101, "", &missing_message),
            temp.path(),
            &validation_profile(),
        );
        assert_eq!(unavailable.status, ProfileStatus::Unavailable);
        let unavailable_reason = unavailable
            .unavailable_reason
            .expect("unavailable profile should explain why");
        assert!(unavailable_reason.contains("<workspace>"));
        assert!(!unavailable_reason.contains(&temp.path().display().to_string()));

        let linker = collect_test_profile(
            &FakeRunner::new(101, "", "error: linking with cc failed"),
            temp.path(),
            &validation_profile(),
        );
        assert_eq!(linker.status, ProfileStatus::Failed);

        let malformed = collect_test_profile(
            &FakeRunner::new(0, "{}", ""),
            temp.path(),
            &validation_profile(),
        );
        assert_eq!(malformed.status, ProfileStatus::Failed);
        assert!(malformed.executable_tests.is_empty());

        let nonzero = collect_test_profile(
            &FakeRunner::new(2, "", "error: test listing failed"),
            temp.path(),
            &validation_profile(),
        );
        assert_eq!(nonzero.status, ProfileStatus::Failed);
        assert!(
            nonzero
                .unavailable_reason
                .expect("failed profile should retain diagnostics")
                .contains("test listing failed")
        );
    }
}
