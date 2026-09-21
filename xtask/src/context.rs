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
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::path::{Path, PathBuf};

mod adapter_docs;
mod collect;
mod evidence;
mod identity;
mod manifest;
mod model;
mod test_inventory;
mod workspace;

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

/// Collects and emits the workspace context snapshot.
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

/// Regenerates the adapter inventory documentation from source metadata.
pub fn sync_adapter_docs(root: &Path) -> Result<()> {
    adapter_docs::sync(root)
}

/// Checks that generated adapter documentation matches source metadata.
pub fn check_adapter_docs(root: &Path) -> Result<()> {
    adapter_docs::check_drift(root)
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
    let identity = collect_repository_identity(runner, repository, &root)?;
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
    let mut workspace = collect_workspace(runner, repository, &root)?;
    attach_workspace_fact_evidence(&mut workspace);
    let source_declarations = test_inventory::collect_source_tests(repository, &root, &workspace)?;

    let native_profile = select_native_profile(&manifest, &environment.target)?;
    let scheduled_profiles =
        profiles_to_collect(&manifest, &native_profile.id, options.validate_all)?;
    let mut scheduled_results = scheduled_profiles
        .into_iter()
        .map(|profile| test_inventory::collect_test_profile(runner, &root, profile));
    let native_result = scheduled_results
        .next()
        .context("native validation profile was not scheduled")?;
    let validate_all_results = scheduled_results.collect::<Vec<_>>();

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
    let workspace_package_names = evidence::workspace_package_identities(&workspace);
    let imported_profiles = match &options.evidence_dir {
        Some(directory) => {
            let directory = if directory.is_absolute() {
                directory.clone()
            } else {
                root.join(directory)
            };
            read_profile_evidence(&directory, &expected_keys, &workspace_package_names)?
        }
        None => Vec::new(),
    };

    // TODO(feature-idea-28): replace declared security and readiness claims with
    // evidence collected from executable checks and validated test profiles.
    // TODO(feature-idea-29): collect sustained concurrency evidence that proves
    // blocking container operations cannot starve Tokio control-plane requests.
    // TODO(feature-idea-30): make doctor evidence cover kernel, cgroups v2,
    // OverlayFS, privileges, and selected-adapter operational prerequisites.
    // TODO(feature-idea-31): benchmark GKE proot startup and copy-filesystem I/O
    // against native overlay and process-spawn baselines.
    // TODO(feature-idea-32): derive production prerequisites and unsupported
    // multi-tenant use cases from executable checks into maintained docs.
    // TODO(feature-idea-33): define evidence thresholds for support-tier
    // graduation using conformance, security, capability, and benchmark results.
    // TODO(feature-idea-34): validate architectural claims about adapter
    // swappability, shared policy boundaries, and capability evidence.
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
    sort_profile_results(&mut profiles);
    let validated_unique_tests = evidence::validated_unique_tests(&profiles);
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
    emit_document_then_enforce(
        output,
        &document,
        options,
        &manifest,
        &native_profile.id,
        &snapshot.tests.profiles,
        &snapshot.diagnostics,
    )?;

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

fn attach_workspace_fact_evidence(workspace: &mut model::WorkspaceSnapshot) {
    for fact in [
        &mut workspace.version,
        &mut workspace.edition,
        &mut workspace.msrv,
    ] {
        if fact.evidence_ids.is_empty() {
            fact.evidence_ids.push(EvidenceId::from("cargo:metadata"));
        }
        fact.evidence_ids.sort();
        fact.evidence_ids.dedup();
    }
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

fn profiles_to_collect<'a>(
    manifest: &'a ContextManifest,
    native_profile_id: &str,
    validate_all: bool,
) -> Result<Vec<&'a ValidationProfile>> {
    let native = manifest
        .profiles
        .iter()
        .find(|profile| profile.id == native_profile_id)
        .with_context(|| format!("native profile {native_profile_id:?} is not configured"))?;
    let mut profiles = vec![native];
    if validate_all {
        let mut additional = manifest
            .profiles
            .iter()
            .filter(|profile| profile.id != native_profile_id)
            .collect::<Vec<_>>();
        additional.sort_by(|left, right| left.id.cmp(&right.id));
        profiles.extend(additional);
    }
    Ok(profiles)
}

fn emit_document_then_enforce(
    output: &impl SnapshotOutput,
    document: &[u8],
    options: &ContextOptions,
    manifest: &ContextManifest,
    native_profile_id: &str,
    profiles: &[TestProfileResult],
    diagnostics: &[ContextDiagnostic],
) -> Result<()> {
    output.write_document(document)?;
    enforce_validation_mode(options, manifest, native_profile_id, profiles, diagnostics)
}

fn enforce_validation_mode(
    options: &ContextOptions,
    manifest: &ContextManifest,
    native_profile_id: &str,
    profiles: &[TestProfileResult],
    diagnostics: &[ContextDiagnostic],
) -> Result<()> {
    if !options.strict {
        return Ok(());
    }
    let required_profiles = if options.validate_all {
        manifest
            .profiles
            .iter()
            .filter(|profile| profile.required_in_ci)
            .map(|profile| profile.id.as_str())
            .collect::<BTreeSet<_>>()
    } else {
        BTreeSet::from([native_profile_id])
    };
    let validated_profiles = profiles
        .iter()
        .filter(|profile| profile.status == ProfileStatus::Validated)
        .map(|profile| profile.profile_id.as_str())
        .collect::<BTreeSet<_>>();
    let missing_profiles = required_profiles
        .difference(&validated_profiles)
        .copied()
        .collect::<Vec<_>>();
    let blocking_diagnostics = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == DiagnosticSeverity::Error)
        .filter(|diagnostic| match diagnostic.profile_id.as_deref() {
            None => true,
            Some(profile_id) => {
                required_profiles.contains(profile_id) && !validated_profiles.contains(profile_id)
            }
        })
        .map(|diagnostic| diagnostic.code.as_str())
        .collect::<Vec<_>>();
    if missing_profiles.is_empty() && blocking_diagnostics.is_empty() {
        Ok(())
    } else {
        bail!(
            "strict context validation failed; missing validated profiles={missing_profiles:?}; diagnostics={blocking_diagnostics:?}"
        )
    }
}

fn sort_profile_results(profiles: &mut [TestProfileResult]) {
    profiles.sort_by(|left, right| {
        left.profile_id
            .cmp(&right.profile_id)
            .then_with(|| {
                profile_status_rank(&left.status).cmp(&profile_status_rank(&right.status))
            })
            .then_with(|| left.target.cmp(&right.target))
            .then_with(|| left.features.cmp(&right.features))
    });
}

const fn profile_status_rank(status: &ProfileStatus) -> u8 {
    match status {
        ProfileStatus::Validated => 0,
        ProfileStatus::Failed => 1,
        ProfileStatus::Unavailable => 2,
        ProfileStatus::Stale => 3,
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
            "profile:test",
            EvidenceKind::Nextest,
            "current profile capability observations",
            EvidenceStatus::Unavailable,
            None,
        ),
        (
            "schema:context-v3",
            EvidenceKind::CiArtifact,
            "xtask/schema/cli.schema.json#/\u{24}defs/contextSnapshot",
            EvidenceStatus::Collected,
            None,
        ),
        (
            "output:context",
            EvidenceKind::CiArtifact,
            "stdout context snapshot",
            EvidenceStatus::Collected,
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
    fn context_v3_collects_complete_snapshot() {
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
                if command.program == "cargo"
                    && command
                        .args
                        .starts_with(&["nextest".to_string(), "list".to_string()])
                    && !command
                        .args
                        .iter()
                        .any(|argument| argument == "aarch64-apple-darwin")
                {
                    return Ok(CommandOutput {
                        exit_code: Some(101),
                        stdout: Vec::new(),
                        stderr: b"error: target may not be installed".to_vec(),
                    });
                }
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

            fn fingerprint_entry(
                &self,
                path: &Path,
                root: &Path,
            ) -> Result<collect::FingerprintEntry> {
                self.inner.fingerprint_entry(path, root)
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
        let mut metadata_value: serde_json::Value =
            serde_json::from_slice(&metadata.stdout).expect("metadata should be JSON");
        let sample_package = metadata_value["packages"]
            .as_array_mut()
            .expect("metadata packages should be an array")
            .iter_mut()
            .find(|package| package["name"] == "sample")
            .expect("sample metadata package should exist");
        sample_package["dependencies"]
            .as_array_mut()
            .expect("sample dependencies should be an array")
            .push(serde_json::json!({
                "name": "sample",
                "source": null,
                "req": "*",
                "kind": "dev",
                "rename": "sample-self",
                "optional": false,
                "uses_default_features": true,
                "features": [],
                "target": null,
                "registry": null,
                "path": root.join("sample")
            }));
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
            metadata: serde_json::to_vec(&metadata_value)
                .expect("captured metadata should serialize"),
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
            &ContextOptions {
                validate_all: true,
                ..ContextOptions::default()
            },
        )
        .expect("default v3 context should collect");

        assert_eq!(snapshot.snapshot_version, 3);
        assert_eq!(snapshot.environment.generated_at, "2026-09-12T00:00:00Z");
        assert_eq!(snapshot.tests.source_declarations.len(), 1);
        assert_eq!(snapshot.tests.profiles.len(), 4);
        assert!(
            snapshot
                .tests
                .profiles
                .iter()
                .any(|profile| profile.profile_id == "native-macos"
                    && profile.status == ProfileStatus::Validated)
        );
        assert_eq!(snapshot.tests.validated_unique_tests.len(), 1);
        assert!(!snapshot.context_map.files.is_empty());
        assert!(!snapshot.context_map.collector_tasks.is_empty());
        assert!(!snapshot.adapters.is_empty());
        assert_ne!(
            snapshot.environment.rustc_version.observed.as_deref(),
            snapshot.workspace.msrv.observed.as_deref(),
            "installed Rust and workspace MSRV must remain separate facts"
        );
        let sample = snapshot
            .workspace
            .packages
            .iter()
            .find(|package| package.name == "sample")
            .expect("sample package should be present");
        assert!(!sample.dependencies.is_empty());
        let dependency_keys = sample
            .dependencies
            .iter()
            .map(|dependency| {
                format!(
                    "{}|{:?}|{:?}|{:?}|{}",
                    dependency.package_name,
                    dependency.rename,
                    dependency.kind,
                    dependency.target_predicate,
                    dependency.optional
                )
            })
            .collect::<BTreeSet<_>>();
        assert_eq!(dependency_keys.len(), sample.dependencies.len());
        assert!(sample.dependencies.iter().any(|dependency| {
            dependency.package_name == "sample" && dependency.kind == model::DependencyKind::Dev
        }));
        assert!(snapshot.adapters.iter().any(|adapter| {
            adapter.id == "native"
                && adapter.maturity.declared == Some(model::AdapterMaturity::Production)
        }));
        assert!(snapshot.adapters.iter().any(|adapter| {
            adapter.id == "smolvm"
                && adapter.maturity.declared == Some(model::AdapterMaturity::Experimental)
        }));
        assert!(
            snapshot
                .tests
                .source_declarations
                .iter()
                .any(|declaration| declaration.name == "works")
        );
        assert!(snapshot.tests.profiles.iter().any(|profile| {
            profile
                .executable_tests
                .iter()
                .any(|test| test.test_name == "tests::works")
        }));
        let unavailable = snapshot
            .tests
            .profiles
            .iter()
            .find(|profile| profile.status == ProfileStatus::Unavailable)
            .expect("foreign unavailable profile should be represented");
        assert!(unavailable.executable_tests.is_empty());
        assert!(unavailable.unavailable_reason.is_some());
        assert!(snapshot.context_map.files.iter().any(|file| {
            file.path.as_str() == "sample/src/lib.rs"
                && file.owner_package_id.as_deref() == Some(sample.package_id.as_str())
        }));

        let serialized = serde_json::to_value(&snapshot).expect("snapshot should serialize");
        let evidence = serialized["evidence"]
            .as_object()
            .expect("evidence should be an object");
        fn assert_fact_evidence_exists(
            value: &serde_json::Value,
            evidence: &serde_json::Map<String, serde_json::Value>,
        ) {
            match value {
                serde_json::Value::Object(object) => {
                    if object.contains_key("declared")
                        && object.contains_key("observed")
                        && object.contains_key("validation")
                    {
                        let ids = object["evidence_ids"]
                            .as_array()
                            .expect("fact evidence IDs should be an array");
                        assert!(!ids.is_empty(), "every fact must cite evidence");
                        for id in ids {
                            assert!(
                                evidence.contains_key(
                                    id.as_str().expect("evidence ID should be a string")
                                ),
                                "fact cites missing evidence ID {id}"
                            );
                        }
                    }
                    for nested in object.values() {
                        assert_fact_evidence_exists(nested, evidence);
                    }
                }
                serde_json::Value::Array(values) => {
                    for nested in values {
                        assert_fact_evidence_exists(nested, evidence);
                    }
                }
                _ => {}
            }
        }
        assert_fact_evidence_exists(&serialized, evidence);
        assert!(snapshot.diagnostics.windows(2).all(|pair| {
            (&pair[0].code, &pair[0].profile_id, &pair[0].message)
                <= (&pair[1].code, &pair[1].profile_id, &pair[1].message)
        }));
        for removed in [
            "crate_assignments",
            "file_assignments",
            "task_slices",
            "ci_workflows",
            "recent_commits",
        ] {
            assert!(!serialized.to_string().contains(removed));
        }
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

        let commands = runner.commands.borrow();
        assert_eq!(
            commands
                .iter()
                .filter(|(program, args)| program == "cargo"
                    && args.starts_with(&["nextest".to_string(), "list".to_string()]))
                .count(),
            4
        );
    }
    #[test]
    fn strict_modes_enforce_their_required_profiles() {
        use model::{ExecutableTest, ProfileStatus};
        use std::cell::RefCell;

        #[derive(Default)]
        struct TestOutput(RefCell<Vec<Vec<u8>>>);

        impl SnapshotOutput for TestOutput {
            fn write_document(&self, document: &[u8]) -> Result<()> {
                self.0.borrow_mut().push(document.to_vec());
                Ok(())
            }
        }

        let profile = |id: &str, required_in_ci: bool| ValidationProfile {
            id: id.to_string(),
            target: format!("target-{id}"),
            features: Vec::new(),
            no_default_features: false,
            all_targets: true,
            required_in_ci,
        };
        let manifest = ContextManifest {
            schema_version: 1,
            adapters: Vec::new(),
            profiles: vec![
                profile("native", true),
                profile("windows", true),
                profile("optional", false),
            ],
        };
        let executable = ExecutableTest {
            stable_id: "pkg::bin::works".to_string(),
            package_id: "pkg".to_string(),
            binary_id: "bin".to_string(),
            test_name: "works".to_string(),
            ignored: false,
        };
        let result =
            |id: &str, status: ProfileStatus, tests: Vec<ExecutableTest>| TestProfileResult {
                profile_id: id.to_string(),
                target: format!("target-{id}"),
                features: Vec::new(),
                status: status.clone(),
                executable_tests: tests,
                unavailable_reason: (status == ProfileStatus::Unavailable)
                    .then(|| "target toolchain is unavailable".to_string()),
                evidence_ids: vec![EvidenceId::from("nextest:list")],
            };
        let native_failed = result("native", ProfileStatus::Failed, Vec::new());
        let diagnostics = vec![ContextDiagnostic {
            code: "native.mismatch".to_string(),
            severity: DiagnosticSeverity::Error,
            message: "native evidence mismatch".to_string(),
            profile_id: Some("native".to_string()),
            evidence_ids: vec![EvidenceId::from("nextest:list")],
        }];
        let output = TestOutput::default();
        emit_document_then_enforce(
            &output,
            b"{\"snapshot_version\":3}\n",
            &ContextOptions::default(),
            &manifest,
            "native",
            std::slice::from_ref(&native_failed),
            &diagnostics,
        )
        .expect("default mode should represent errors without failing");
        assert_eq!(output.0.borrow().len(), 1);

        let strict = ContextOptions {
            strict: true,
            ..ContextOptions::default()
        };
        assert!(
            emit_document_then_enforce(
                &output,
                b"{\"snapshot_version\":3}\n",
                &strict,
                &manifest,
                "native",
                std::slice::from_ref(&native_failed),
                &diagnostics,
            )
            .is_err()
        );
        assert_eq!(
            output.0.borrow().len(),
            2,
            "JSON must be emitted before strict failure"
        );

        let native_zero = result("native", ProfileStatus::Validated, Vec::new());
        emit_document_then_enforce(
            &output,
            b"{\"snapshot_version\":3}\n",
            &strict,
            &manifest,
            "native",
            std::slice::from_ref(&native_zero),
            &[],
        )
        .expect("a validated empty target is distinct from an unavailable target");
        let unavailable = result("native", ProfileStatus::Unavailable, Vec::new());
        assert_eq!(unavailable.status, ProfileStatus::Unavailable);
        assert!(unavailable.unavailable_reason.is_some());
        assert!(
            enforce_validation_mode(
                &strict,
                &manifest,
                "native",
                std::slice::from_ref(&unavailable),
                &[],
            )
            .is_err()
        );

        assert_eq!(
            profiles_to_collect(&manifest, "native", false)
                .expect("default schedule should resolve")
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<Vec<_>>(),
            ["native"]
        );
        assert_eq!(
            profiles_to_collect(&manifest, "native", true)
                .expect("full schedule should resolve")
                .iter()
                .map(|profile| profile.id.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["native", "optional", "windows"])
        );

        let key = ProfileEvidenceCacheKey {
            commit: "abc123".to_string(),
            worktree_fingerprint: "clean".to_string(),
            manifest_sha256: "manifest".to_string(),
            target: "target-windows".to_string(),
            features: Vec::new(),
            no_default_features: false,
            cargo_version: "cargo 1.85.0".to_string(),
            rustc_version: "rustc 1.85.0".to_string(),
            nextest_version: "cargo-nextest 0.9".to_string(),
        };
        let artifact = |cache_key: &ProfileEvidenceCacheKey| {
            serde_json::json!({
                "schema_version": 1,
                "profile_id": "windows",
                "cache_key": cache_key,
                "status": "validated",
                "executable_tests": [{
                    "stable_id": executable.stable_id,
                    "package_id": executable.package_id,
                    "binary_id": executable.binary_id,
                    "test_name": executable.test_name,
                    "ignored": executable.ignored
                }],
                "unavailable_reason": null
            })
        };
        let expected = BTreeMap::from([("windows".to_string(), key.clone())]);
        let current_dir = tempfile::tempdir().expect("current evidence directory should exist");
        std::fs::write(
            current_dir.path().join("windows.json"),
            serde_json::to_vec(&artifact(&key)).expect("current artifact should serialize"),
        )
        .expect("current artifact should be written");
        let packages = BTreeSet::from(["pkg".to_string()]);
        let current = read_profile_evidence(current_dir.path(), &expected, &packages)
            .expect("exact-key evidence should import");
        assert_eq!(current[0].status, ProfileStatus::Validated);

        let stale_dir = tempfile::tempdir().expect("stale evidence directory should exist");
        let mut stale_key = key.clone();
        stale_key.commit = "different".to_string();
        std::fs::write(
            stale_dir.path().join("windows.json"),
            serde_json::to_vec(&artifact(&stale_key)).expect("stale artifact should serialize"),
        )
        .expect("stale artifact should be written");
        let stale = read_profile_evidence(stale_dir.path(), &expected, &packages)
            .expect("stale evidence should remain diagnostic data");
        assert_eq!(stale[0].status, ProfileStatus::Stale);
        assert!(stale[0].executable_tests.is_empty());

        let validate_all_strict = ContextOptions {
            strict: true,
            validate_all: true,
            ..ContextOptions::default()
        };
        let local_results = vec![
            result("native", ProfileStatus::Validated, vec![executable.clone()]),
            result("windows", ProfileStatus::Unavailable, Vec::new()),
            result("optional", ProfileStatus::Unavailable, Vec::new()),
        ];
        let mut with_current = local_results.clone();
        with_current.extend(current);
        enforce_validation_mode(
            &validate_all_strict,
            &manifest,
            "native",
            &with_current,
            &[],
        )
        .expect("exact-key CI evidence should satisfy a required profile");
        let mut with_stale = local_results;
        with_stale.extend(stale);
        assert!(
            enforce_validation_mode(&validate_all_strict, &manifest, "native", &with_stale, &[],)
                .is_err()
        );

        let deduplicated = evidence::validated_unique_tests(&with_current);
        assert_eq!(deduplicated, [executable]);
    }

    #[test]
    fn required_tool_collectors_propagate_status_utf8_and_shape_failures() {
        struct Runner(CommandOutput);
        impl CommandRunner for Runner {
            fn run(&self, _command: &CommandSpec) -> Result<CommandOutput> {
                Ok(self.0.clone())
            }
        }
        let root = Path::new(".");
        let nonzero = Runner(CommandOutput {
            exit_code: Some(1),
            stdout: Vec::new(),
            stderr: b"required tool failed".to_vec(),
        });
        let error = run_text_command(&nonzero, root, "cargo", &["--version"])
            .expect_err("nonzero required collector must fail");
        assert!(error.to_string().contains("required tool failed"));

        let invalid_utf8 = Runner(CommandOutput {
            exit_code: Some(0),
            stdout: vec![0xff],
            stderr: Vec::new(),
        });
        assert!(run_text_command(&invalid_utf8, root, "cargo", &["--version"]).is_err());

        let missing_host = Runner(CommandOutput {
            exit_code: Some(0),
            stdout: b"rustc 1.85.0\n".to_vec(),
            stderr: Vec::new(),
        });
        assert!(collect_environment(&missing_host, root, "now").is_err());
    }
}
