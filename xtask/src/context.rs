//! `cargo xtask info context` — evidence-backed repository context.

use anyhow::{Context, Result, bail};
use chrono::{SecondsFormat, Utc};
use collect::output::{
    PendingProfileEvidence, ProfileEvidenceCacheKey, persist_snapshot, read_profile_evidence,
    validate_snapshot_json,
};
use collect::{
    CollectorExecution, CommandOutput, CommandRunner, CommandSpec, RepositoryReader,
    SystemCommandRunner, SystemRepositoryReader, collect_repository_identity, collect_workspace,
    derive_collector_tasks, derive_context_map, reconcile_adapters,
};
use manifest::{ContextManifest, ValidationProfile, parse_adapter_registry, validate_manifest};
use model::{
    ContextDiagnostic, ContextSnapshot, DiagnosticSeverity, EnvironmentSnapshot, Evidence,
    EvidenceId, EvidenceKind, EvidenceStatus, Fact, ProfileStatus, TestProfileResult, TestSnapshot,
    Validation, ValidationState,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};

#[allow(dead_code)]
mod collect;
#[allow(dead_code)]
mod manifest;
#[allow(dead_code)]
mod model;
#[allow(dead_code)]
mod test_inventory;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContextOptions {
    pub save: bool,
    pub strict: bool,
    pub validate_all: bool,
    pub evidence_dir: Option<PathBuf>,
}

trait SnapshotOutput {
    fn write_document(&self, document: &[u8]) -> Result<()>;
}

trait Clock {
    fn now(&self) -> String;
}

struct StdoutOutput;

impl SnapshotOutput for StdoutOutput {
    fn write_document(&self, document: &[u8]) -> Result<()> {
        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        stdout
            .write_all(document)
            .context("write context JSON to stdout")?;
        stdout.flush().context("flush context JSON stdout")
    }
}

struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> String {
        Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true)
    }
}

pub fn context(root: &Path, options: &ContextOptions) -> Result<()> {
    run_context(
        &SystemCommandRunner,
        &SystemRepositoryReader,
        &StdoutOutput,
        &SystemClock,
        root,
        options,
    )?;
    Ok(())
}

fn run_context(
    runner: &impl CommandRunner,
    repository: &impl RepositoryReader,
    output: &impl SnapshotOutput,
    clock: &impl Clock,
    root: &Path,
    options: &ContextOptions,
) -> Result<ContextSnapshot> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize workspace root {}", root.display()))?;
    let generated_at = clock.now();
    let identity = collect_repository_identity(runner, &root)?;
    let environment = collect_environment(runner, &root, &generated_at)?;

    let manifest_path = root.join("xtask/context.toml");
    let manifest_source = repository.read_utf8(&manifest_path)?;
    let manifest: ContextManifest =
        toml::from_str(&manifest_source).context("parse xtask/context.toml")?;
    validate_manifest(&manifest)?;
    let manifest_sha256 = hex::encode(Sha256::digest(manifest_source.as_bytes()));

    let registry_source =
        repository.read_utf8(&root.join("crates/miniboxd/src/adapter_registry.rs"))?;
    let registry = parse_adapter_registry(&registry_source)?;
    let workspace = collect_workspace(runner, repository, &root)?;
    let source_declarations = test_inventory::collect_source_tests(repository, &root, &workspace)?;

    let native_profile = select_native_profile(&manifest, &environment.target)?;
    let native_result = test_inventory::collect_test_profile(runner, &root, native_profile);
    let mut validate_all_results = Vec::new();
    if options.validate_all {
        for profile in &manifest.profiles {
            if profile.id != native_profile.id {
                validate_all_results
                    .push(test_inventory::collect_test_profile(runner, &root, profile));
            }
        }
    }

    let expected_keys = manifest
        .profiles
        .iter()
        .map(|profile| {
            Ok((
                profile.id.clone(),
                profile_cache_key(profile, &identity, &environment, &manifest_sha256)?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>>>()?;
    let imported_profiles = match &options.evidence_dir {
        Some(directory) => {
            let directory = if directory.is_absolute() {
                directory.clone()
            } else {
                root.join(directory)
            };
            read_profile_evidence(&directory, &expected_keys)?
        }
        None => Vec::new(),
    };

    let tested_capabilities = BTreeMap::new();
    let reconciliation = reconcile_adapters(&manifest, &registry, &tested_capabilities);
    let mut diagnostics = reconciliation.diagnostics;
    for profile in std::iter::once(&native_result)
        .chain(validate_all_results.iter())
        .chain(imported_profiles.iter())
    {
        if profile.status != ProfileStatus::Validated {
            diagnostics.push(profile_diagnostic(profile));
        }
    }
    sort_diagnostics(&mut diagnostics);

    let mut profiles = vec![native_result.clone()];
    profiles.extend(validate_all_results.iter().cloned());
    profiles.extend(imported_profiles.iter().cloned());
    let validated_unique_tests = validated_unique_tests(&profiles);
    let tests = TestSnapshot {
        source_declarations,
        profiles,
        validated_unique_tests,
    };

    let mut context_map =
        derive_context_map(repository, &root, &workspace, &identity.changed_paths)?;
    context_map.collector_tasks = derive_collector_tasks(&CollectorExecution {
        native_profile: native_result.clone(),
        validate_all_profiles: validate_all_results.clone(),
        imported_profiles: imported_profiles.clone(),
        outcomes: BTreeMap::new(),
    })?;

    let evidence = build_evidence(
        &generated_at,
        &manifest_sha256,
        &native_result,
        &environment,
    );
    let snapshot = ContextSnapshot {
        snapshot_version: 3,
        identity,
        environment,
        workspace,
        adapters: reconciliation.adapters,
        tests,
        context_map,
        evidence,
        diagnostics,
    };
    let value = serde_json::to_value(&snapshot).context("serialize context snapshot")?;
    validate_snapshot_json(&value)?;
    let mut document = serde_json::to_vec_pretty(&value).context("format context JSON")?;
    document.push(b'\n');
    output.write_document(&document)?;

    let mut pending_evidence = Vec::new();
    for result in std::iter::once(&native_result).chain(validate_all_results.iter()) {
        let key = expected_keys
            .get(&result.profile_id)
            .with_context(|| format!("profile {:?} has no evidence key", result.profile_id))?;
        pending_evidence.push(PendingProfileEvidence {
            expected_key: key.clone(),
            actual_key: key.clone(),
            result: result.clone(),
        });
    }
    persist_snapshot(&root, &value, options.save, &pending_evidence)?;
    Ok(snapshot)
}

fn collect_environment(
    runner: &impl CommandRunner,
    root: &Path,
    generated_at: &str,
) -> Result<EnvironmentSnapshot> {
    let rustc = run_text_command(runner, root, "rustc", &["-vV"])?;
    let rustc_version = rustc
        .lines()
        .next()
        .filter(|line| !line.is_empty())
        .context("rustc -vV omitted its version")?
        .to_string();
    let host = rustc
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .filter(|host| !host.is_empty())
        .context("rustc -vV omitted its host target")?
        .to_string();
    let cargo_version = run_text_command(runner, root, "cargo", &["--version"])?;
    let nextest_version = run_text_command(runner, root, "cargo", &["nextest", "--version"])?;
    Ok(EnvironmentSnapshot {
        host: host.clone(),
        target: host,
        enabled_features: Vec::new(),
        rustc_version: observed_fact(rustc_version, "tool:rustc"),
        cargo_version: observed_fact(cargo_version.trim().to_string(), "tool:cargo"),
        nextest_version: observed_fact(nextest_version.trim().to_string(), "tool:nextest"),
        generated_at: generated_at.to_string(),
    })
}

fn run_text_command(
    runner: &impl CommandRunner,
    root: &Path,
    program: &str,
    args: &[&str],
) -> Result<String> {
    let output = runner.run(&CommandSpec {
        program: program.to_string(),
        args: args.iter().map(ToString::to_string).collect(),
        current_dir: root.to_path_buf(),
    })?;
    require_success(program, args, &output)?;
    String::from_utf8(output.stdout)
        .with_context(|| format!("{program} {} output is not UTF-8", args.join(" ")))
}

fn require_success(program: &str, args: &[&str], output: &CommandOutput) -> Result<()> {
    if output.exit_code == Some(0) {
        return Ok(());
    }
    bail!(
        "{program} {} failed with status {:?}: {}",
        args.join(" "),
        output.exit_code,
        String::from_utf8_lossy(&output.stderr).trim()
    )
}

fn observed_fact(value: String, evidence_id: &'static str) -> Fact<String> {
    Fact::new(
        None,
        Some(value),
        Validation {
            state: ValidationState::ObservedOnly,
            reason: None,
        },
        vec![EvidenceId::from(evidence_id)],
    )
}

fn select_native_profile<'a>(
    manifest: &'a ContextManifest,
    host: &str,
) -> Result<&'a ValidationProfile> {
    let profiles = manifest
        .profiles
        .iter()
        .filter(|profile| {
            profile.target == host && profile.features.is_empty() && !profile.no_default_features
        })
        .collect::<Vec<_>>();
    match profiles.as_slice() {
        [profile] => Ok(profile),
        [] => bail!("no native validation profile is configured for host {host:?}"),
        _ => bail!("multiple native validation profiles are configured for host {host:?}"),
    }
}

fn profile_cache_key(
    profile: &ValidationProfile,
    identity: &model::RepositoryIdentity,
    environment: &EnvironmentSnapshot,
    manifest_sha256: &str,
) -> Result<ProfileEvidenceCacheKey> {
    Ok(ProfileEvidenceCacheKey {
        commit: identity.commit.clone(),
        worktree_fingerprint: identity.worktree_fingerprint.clone(),
        manifest_sha256: manifest_sha256.to_string(),
        target: profile.target.clone(),
        features: profile.features.clone(),
        no_default_features: profile.no_default_features,
        cargo_version: observed_value(&environment.cargo_version, "cargo version")?,
        rustc_version: observed_value(&environment.rustc_version, "rustc version")?,
        nextest_version: observed_value(&environment.nextest_version, "nextest version")?,
    })
}

fn observed_value(fact: &Fact<String>, name: &str) -> Result<String> {
    fact.observed
        .clone()
        .with_context(|| format!("{name} was not observed"))
}

fn validated_unique_tests(profiles: &[TestProfileResult]) -> Vec<model::ExecutableTest> {
    let mut tests = profiles
        .iter()
        .filter(|profile| profile.status == ProfileStatus::Validated)
        .flat_map(|profile| profile.executable_tests.iter().cloned())
        .collect::<Vec<_>>();
    tests.sort_by(|left, right| left.stable_id.cmp(&right.stable_id));
    tests.dedup_by(|left, right| left.stable_id == right.stable_id);
    tests
}

fn profile_diagnostic(profile: &TestProfileResult) -> ContextDiagnostic {
    let severity = match profile.status {
        ProfileStatus::Validated => DiagnosticSeverity::Info,
        ProfileStatus::Failed => DiagnosticSeverity::Error,
        ProfileStatus::Unavailable | ProfileStatus::Stale => DiagnosticSeverity::Warning,
    };
    ContextDiagnostic {
        code: format!("profile.{:?}", profile.status).to_ascii_lowercase(),
        severity,
        message: profile
            .unavailable_reason
            .clone()
            .unwrap_or_else(|| format!("profile {:?} is {:?}", profile.profile_id, profile.status)),
        profile_id: Some(profile.profile_id.clone()),
        evidence_ids: profile.evidence_ids.clone(),
    }
}

fn sort_diagnostics(diagnostics: &mut [ContextDiagnostic]) {
    diagnostics.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| left.profile_id.cmp(&right.profile_id))
            .then_with(|| left.message.cmp(&right.message))
    });
}

fn build_evidence(
    generated_at: &str,
    manifest_sha256: &str,
    native_profile: &TestProfileResult,
    environment: &EnvironmentSnapshot,
) -> BTreeMap<EvidenceId, Evidence> {
    let profile_status = match native_profile.status {
        ProfileStatus::Validated => EvidenceStatus::Collected,
        ProfileStatus::Failed => EvidenceStatus::Failed,
        ProfileStatus::Unavailable => EvidenceStatus::Unavailable,
        ProfileStatus::Stale => EvidenceStatus::Stale,
    };
    [
        (
            "git:branch",
            EvidenceKind::Git,
            "git branch --show-current",
            EvidenceStatus::Collected,
            None,
        ),
        (
            "git:head",
            EvidenceKind::Git,
            "git rev-parse HEAD",
            EvidenceStatus::Collected,
            None,
        ),
        (
            "git:status",
            EvidenceKind::Git,
            "git status --porcelain=v2 -z --untracked-files=all",
            EvidenceStatus::Collected,
            None,
        ),
        (
            "git:tracked-files",
            EvidenceKind::Git,
            "git ls-files -z",
            EvidenceStatus::Collected,
            None,
        ),
        (
            "manifest:adapter",
            EvidenceKind::Manifest,
            "xtask/context.toml",
            EvidenceStatus::Collected,
            Some(manifest_sha256.to_string()),
        ),
        (
            "cargo:metadata",
            EvidenceKind::CargoMetadata,
            "cargo metadata --format-version 1",
            EvidenceStatus::Collected,
            None,
        ),
        (
            "source:adapter_registry",
            EvidenceKind::RustSource,
            "crates/miniboxd/src/adapter_registry.rs",
            EvidenceStatus::Collected,
            None,
        ),
        (
            "source:tests",
            EvidenceKind::RustSource,
            "tracked Rust test declarations",
            EvidenceStatus::Collected,
            None,
        ),
        (
            "nextest:list",
            EvidenceKind::Nextest,
            "cargo nextest list --message-format json",
            profile_status,
            None,
        ),
        (
            "tool:rustc",
            EvidenceKind::ToolVersion,
            environment
                .rustc_version
                .observed
                .as_deref()
                .expect("observed rustc fact is constructed above"),
            EvidenceStatus::Collected,
            None,
        ),
        (
            "tool:cargo",
            EvidenceKind::ToolVersion,
            environment
                .cargo_version
                .observed
                .as_deref()
                .expect("observed cargo fact is constructed above"),
            EvidenceStatus::Collected,
            None,
        ),
        (
            "tool:nextest",
            EvidenceKind::ToolVersion,
            environment
                .nextest_version
                .observed
                .as_deref()
                .expect("observed nextest fact is constructed above"),
            EvidenceStatus::Collected,
            None,
        ),
    ]
    .into_iter()
    .map(|(id, kind, locator, status, content_sha256)| {
        (
            EvidenceId::from(id),
            Evidence {
                kind,
                locator: locator.to_string(),
                collected_at: generated_at.to_string(),
                content_sha256,
                profile_id: (id == "nextest:list").then(|| native_profile.profile_id.clone()),
                status,
            },
        )
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_context_collects_native_evidence() {
        use collect::{
            CommandOutput, CommandRunner, CommandSpec, RepositoryReader, SystemRepositoryReader,
        };
        use model::{FileMetrics, RepositoryPath};
        use std::cell::RefCell;

        struct FakeRunner {
            metadata: Vec<u8>,
            nextest: Vec<u8>,
            commands: RefCell<Vec<(String, Vec<String>)>>,
        }

        impl CommandRunner for FakeRunner {
            fn run(&self, command: &CommandSpec) -> Result<CommandOutput> {
                self.commands
                    .borrow_mut()
                    .push((command.program.clone(), command.args.clone()));
                let stdout = match (command.program.as_str(), command.args.as_slice()) {
                    ("git", args) if args == ["rev-parse", "HEAD"] => b"abc123\n".to_vec(),
                    ("git", args) if args == ["branch", "--show-current"] => b"develop\n".to_vec(),
                    ("git", args) if args.first().map(String::as_str) == Some("status") => {
                        Vec::new()
                    }
                    ("rustc", args) if args == ["-vV"] => {
                        b"rustc 1.85.0\nhost: aarch64-apple-darwin\n".to_vec()
                    }
                    ("cargo", args) if args == ["--version"] => b"cargo 1.85.0\n".to_vec(),
                    ("cargo", args) if args == ["nextest", "--version"] => {
                        b"cargo-nextest 0.9.100\n".to_vec()
                    }
                    ("cargo", args) if args.first().map(String::as_str) == Some("metadata") => {
                        self.metadata.clone()
                    }
                    ("cargo", args)
                        if args.starts_with(&["nextest".to_string(), "list".to_string()]) =>
                    {
                        self.nextest.clone()
                    }
                    other => panic!("unexpected command: {other:?}"),
                };
                Ok(CommandOutput {
                    exit_code: Some(0),
                    stdout,
                    stderr: Vec::new(),
                })
            }
        }

        struct CountingRepository {
            inner: SystemRepositoryReader,
            reads: RefCell<BTreeMap<String, usize>>,
        }

        impl RepositoryReader for CountingRepository {
            fn read_utf8(&self, path: &Path) -> Result<String> {
                *self
                    .reads
                    .borrow_mut()
                    .entry(path.to_string_lossy().into_owned())
                    .or_default() += 1;
                self.inner.read_utf8(path)
            }

            fn tracked_files(&self, root: &Path) -> Result<Vec<RepositoryPath>> {
                self.inner.tracked_files(root)
            }

            fn file_metrics(&self, path: &Path) -> Result<FileMetrics> {
                self.inner.file_metrics(path)
            }
        }

        #[derive(Default)]
        struct BufferOutput(RefCell<Vec<Vec<u8>>>);

        impl SnapshotOutput for BufferOutput {
            fn write_document(&self, document: &[u8]) -> Result<()> {
                self.0.borrow_mut().push(document.to_vec());
                Ok(())
            }
        }

        struct FixedClock;

        impl Clock for FixedClock {
            fn now(&self) -> String {
                "2026-09-12T00:00:00Z".to_string()
            }
        }

        let temp = tempfile::tempdir().expect("temporary workspace should be created");
        let root = temp.path();
        for directory in ["sample/src", "xtask", "crates/miniboxd/src"] {
            std::fs::create_dir_all(root.join(directory))
                .expect("fixture directory should be created");
        }
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
name = "sample"
version = "0.1.0"
edition = "2024"
rust-version = "1.85"
"#,
        )
        .expect("sample manifest should be written");
        std::fs::write(
            root.join("sample/src/lib.rs"),
            "#[cfg(test)] mod tests { #[test] fn works() {} }\n",
        )
        .expect("sample source should be written");
        std::fs::write(
            root.join("xtask/context.toml"),
            r#"schema_version = 1
[[adapters]]
id = "native"
maturity = "production"
platforms = ["linux"]
default_roles = ["linux_fallback"]
[adapters.capabilities]
run = "yes"
[[adapters]]
id = "smolvm"
maturity = "experimental"
platforms = ["macos"]
default_roles = ["unix_default"]
[adapters.capabilities]
run = "yes"
[[adapters]]
id = "krun"
maturity = "experimental"
platforms = ["macos"]
default_roles = ["macos_fallback"]
[adapters.capabilities]
run = "yes"
[[profiles]]
id = "native-macos"
target = "aarch64-apple-darwin"
features = []
no_default_features = false
all_targets = true
required_in_ci = true
[[profiles]]
id = "native-linux-gnu"
target = "x86_64-unknown-linux-gnu"
features = []
no_default_features = false
all_targets = true
required_in_ci = true
[[profiles]]
id = "native-linux-musl"
target = "x86_64-unknown-linux-musl"
features = []
no_default_features = false
all_targets = true
required_in_ci = true
[[profiles]]
id = "native-windows"
target = "x86_64-pc-windows-msvc"
features = []
no_default_features = false
all_targets = true
required_in_ci = true
"#,
        )
        .expect("context manifest should be written");
        std::fs::write(
            root.join("crates/miniboxd/src/adapter_registry.rs"),
            r#"pub enum AdapterSuite { Native, SmolVm, Krun }
impl AdapterSuite { pub const fn as_str(&self) -> &str { match self { Self::Native => "native", Self::SmolVm => "smolvm", Self::Krun => "krun" } } }
pub const VALID_ADAPTERS: &[&str] = &["native", "smolvm", "krun"];
pub const DEFAULT_ADAPTER_SUITE: &str = "smolvm";
pub const FALLBACK_ADAPTER_SUITE: &str = if cfg!(target_os = "linux") { "native" } else { "krun" };
pub fn all_adapters() -> Vec<AdapterInfo> { vec![AdapterInfo { name: "native", available: true, platform: "linux" }, AdapterInfo { name: "smolvm", available: true, platform: "macos" }, AdapterInfo { name: "krun", available: true, platform: "macos" }] }
"#,
        )
        .expect("adapter registry should be written");
        for args in [vec!["init", "--quiet"], vec!["add", "--all"]] {
            let status = std::process::Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .expect("git fixture command should run");
            assert!(status.success(), "git fixture command should succeed");
        }
        let metadata = std::process::Command::new("cargo")
            .args(["metadata", "--format-version", "1"])
            .current_dir(root)
            .output()
            .expect("fixture cargo metadata should run");
        assert!(metadata.status.success());
        let metadata_value: serde_json::Value =
            serde_json::from_slice(&metadata.stdout).expect("metadata should be JSON");
        let package_id = metadata_value["workspace_members"][0]
            .as_str()
            .expect("workspace package id should be present");
        let nextest = serde_json::to_vec(&serde_json::json!({
            "rust-build-meta": {},
            "test-count": 1,
            "rust-suites": {
                "sample::lib": {
                    "package-name": "sample",
                    "binary-id": "sample::lib",
                    "binary-name": "sample",
                    "package-id": package_id,
                    "kind": "lib",
                    "binary-path": "/target/sample",
                    "build-platform": "target",
                    "cwd": "/workspace/sample",
                    "status": "listed",
                    "testcases": {
                        "tests::works": {
                            "kind": "test",
                            "ignored": false,
                            "filter-match": { "status": "matches" }
                        }
                    }
                }
            }
        }))
        .expect("nextest fixture should serialize");
        let runner = FakeRunner {
            metadata: metadata.stdout,
            nextest,
            commands: RefCell::new(Vec::new()),
        };
        let repository = CountingRepository {
            inner: SystemRepositoryReader,
            reads: RefCell::new(BTreeMap::new()),
        };
        let output = BufferOutput::default();
        let canonical_root = root
            .canonicalize()
            .expect("fixture workspace root should canonicalize");
        let snapshot = run_context(
            &runner,
            &repository,
            &output,
            &FixedClock,
            root,
            &ContextOptions::default(),
        )
        .expect("default v3 context should collect");

        assert_eq!(snapshot.snapshot_version, 3);
        assert_eq!(snapshot.environment.generated_at, "2026-09-12T00:00:00Z");
        assert_eq!(snapshot.tests.source_declarations.len(), 1);
        assert_eq!(snapshot.tests.profiles.len(), 1);
        assert_eq!(snapshot.tests.profiles[0].profile_id, "native-macos");
        assert_eq!(snapshot.tests.validated_unique_tests.len(), 1);
        assert!(!snapshot.context_map.files.is_empty());
        assert!(!snapshot.context_map.collector_tasks.is_empty());
        assert!(!snapshot.adapters.is_empty());
        assert_eq!(
            repository.reads.borrow().get(
                &canonical_root
                    .join("xtask/context.toml")
                    .to_string_lossy()
                    .into_owned(),
            ),
            Some(&1)
        );
        let documents = output.0.borrow();
        assert_eq!(documents.len(), 1);
        let emitted: serde_json::Value =
            serde_json::from_slice(&documents[0]).expect("stdout should be one JSON document");
        assert_eq!(emitted["snapshot_version"], 3);
        assert!(!root.join("artifacts/context").exists());

        assert_eq!(
            runner.commands.borrow().as_slice(),
            [
                (
                    "git".to_string(),
                    vec!["rev-parse".to_string(), "HEAD".to_string()]
                ),
                (
                    "git".to_string(),
                    vec!["branch".to_string(), "--show-current".to_string()]
                ),
                (
                    "git".to_string(),
                    vec![
                        "status".to_string(),
                        "--porcelain=v2".to_string(),
                        "-z".to_string(),
                        "--untracked-files=all".to_string(),
                    ],
                ),
                ("rustc".to_string(), vec!["-vV".to_string()]),
                ("cargo".to_string(), vec!["--version".to_string()]),
                (
                    "cargo".to_string(),
                    vec!["nextest".to_string(), "--version".to_string()],
                ),
                (
                    "cargo".to_string(),
                    vec![
                        "metadata".to_string(),
                        "--format-version".to_string(),
                        "1".to_string(),
                    ],
                ),
                (
                    "cargo".to_string(),
                    vec![
                        "nextest".to_string(),
                        "list".to_string(),
                        "--workspace".to_string(),
                        "--message-format".to_string(),
                        "json".to_string(),
                        "--target".to_string(),
                        "aarch64-apple-darwin".to_string(),
                        "--all-targets".to_string(),
                    ],
                ),
            ]
        );
    }
}
