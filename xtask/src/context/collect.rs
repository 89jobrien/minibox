use super::manifest::{AdapterRegistryObservation, ContextManifest};
use super::model::{
    AdapterMaturity, AdapterSnapshot, CapabilitySupport, ContextDiagnostic, DependencyKind,
    DependencySnapshot, DiagnosticSeverity, EvidenceId, Fact, FileMetrics, PackageSnapshot,
    RepositoryIdentity, RepositoryPath, SourceMetrics, TargetSnapshot, Validation, ValidationState,
    WorkspaceSnapshot,
};
use anyhow::{Context, Result, bail};
use cargo_metadata::{DependencyKind as CargoDependencyKind, MetadataCommand};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "output.rs"]
pub(super) mod output;

pub(super) trait CommandRunner {
    fn run(&self, command: &CommandSpec) -> Result<CommandOutput>;
}

pub(super) trait RepositoryReader {
    fn read_utf8(&self, path: &Path) -> Result<String>;
    fn tracked_files(&self, root: &Path) -> Result<Vec<RepositoryPath>>;
    fn file_metrics(&self, path: &Path) -> Result<FileMetrics>;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CommandSpec {
    pub(super) program: String,
    pub(super) args: Vec<String>,
    pub(super) current_dir: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct CommandOutput {
    pub(super) exit_code: Option<i32>,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
}

pub(super) fn redact_diagnostic(root: &Path, diagnostic: &str) -> String {
    let mut redacted = diagnostic.to_string();
    let canonical_root = root.canonicalize().ok();
    let mut paths = vec![root.to_path_buf()];
    if let Some(canonical) = &canonical_root {
        paths.push(canonical.clone());
    }
    if let Some(home) = std::env::var_os("HOME") {
        paths.push(PathBuf::from(home));
    }
    paths.sort_by_key(|path| std::cmp::Reverse(path.as_os_str().len()));
    paths.dedup();
    for path in paths {
        let value = path.to_string_lossy();
        if !value.is_empty() && value != "/" {
            let replacement = if path == root || canonical_root.as_ref() == Some(&path) {
                "<workspace>"
            } else {
                "<home>"
            };
            redacted = redacted.replace(value.as_ref(), replacement);
        }
    }

    let redacted = redacted
        .split_inclusive(char::is_whitespace)
        .map(|part| {
            let token = part.trim_end_matches(char::is_whitespace);
            let suffix = &part[token.len()..];
            let Some((key, _)) = token.split_once('=') else {
                return part.to_string();
            };
            let key_upper = key.to_ascii_uppercase();
            if ["TOKEN", "PASSWORD", "SECRET", "API_KEY"]
                .iter()
                .any(|marker| key_upper.contains(marker))
            {
                format!("{key}=<redacted>{suffix}")
            } else {
                part.to_string()
            }
        })
        .collect::<String>();
    redacted.chars().take(4096).collect()
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, command: &CommandSpec) -> Result<CommandOutput> {
        let output = Command::new(&command.program)
            .args(&command.args)
            .current_dir(&command.current_dir)
            .output()
            .with_context(|| {
                format!(
                    "run command {} in {}",
                    command.program,
                    command.current_dir.display()
                )
            })?;

        Ok(CommandOutput {
            exit_code: output.status.code(),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) struct SystemRepositoryReader;

impl RepositoryReader for SystemRepositoryReader {
    fn read_utf8(&self, path: &Path) -> Result<String> {
        std::fs::read_to_string(path).with_context(|| format!("read UTF-8 file {}", path.display()))
    }

    fn tracked_files(&self, root: &Path) -> Result<Vec<RepositoryPath>> {
        let output = Command::new("git")
            .args(["-C", &root.to_string_lossy(), "ls-files", "-z", "--"])
            .output()
            .with_context(|| format!("list tracked files in {}", root.display()))?;
        if !output.status.success() {
            bail!(
                "git ls-files failed in {} with status {:?}",
                root.display(),
                output.status.code()
            );
        }

        let mut paths = output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| {
                let relative = std::str::from_utf8(path)
                    .with_context(|| format!("tracked path in {} is not UTF-8", root.display()))?;
                RepositoryPath::from_root(root, &root.join(relative))
            })
            .collect::<Result<Vec<_>>>()?;
        paths.sort();
        paths.dedup();
        Ok(paths)
    }

    fn file_metrics(&self, path: &Path) -> Result<FileMetrics> {
        let bytes = std::fs::read(path).with_context(|| format!("read file {}", path.display()))?;
        let content = std::str::from_utf8(&bytes)
            .with_context(|| format!("file is not valid UTF-8: {}", path.display()))?;
        let physical_lines = content.lines().count();
        let non_empty_lines = content
            .lines()
            .filter(|line| !line.trim().is_empty())
            .count();
        let content_sha256 = hex::encode(Sha256::digest(&bytes));

        Ok(FileMetrics {
            physical_lines,
            non_empty_lines,
            content_sha256,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct AdapterReconciliation {
    pub(super) adapters: Vec<AdapterSnapshot>,
    pub(super) diagnostics: Vec<ContextDiagnostic>,
}

pub(super) fn reconcile_adapters(
    manifest: &ContextManifest,
    registry: &AdapterRegistryObservation,
    tested_capabilities: &BTreeMap<String, BTreeMap<String, CapabilitySupport>>,
) -> AdapterReconciliation {
    let manifest_ids = manifest
        .adapters
        .iter()
        .map(|adapter| adapter.id.as_str())
        .collect::<BTreeSet<_>>();
    let mut diagnostics = Vec::new();
    let mut adapters = manifest
        .adapters
        .iter()
        .map(|adapter| {
            reconcile_adapter(
                adapter,
                registry,
                tested_capabilities.get(&adapter.id),
                &mut diagnostics,
            )
        })
        .collect::<Vec<_>>();
    for extra in registry
        .adapter_ids
        .iter()
        .filter(|adapter| !manifest_ids.contains(adapter.as_str()))
    {
        diagnostics.push(adapter_diagnostic(
            "adapter.registry_extra",
            DiagnosticSeverity::Error,
            format!("adapter {extra:?} is observed in adapter_registry.rs but not declared"),
        ));
    }
    adapters.sort_by(|left, right| left.id.cmp(&right.id));
    diagnostics.sort_by(|left, right| {
        left.code
            .cmp(&right.code)
            .then_with(|| left.message.cmp(&right.message))
    });
    AdapterReconciliation {
        adapters,
        diagnostics,
    }
}

fn reconcile_adapter(
    declaration: &super::manifest::AdapterDeclaration,
    registry: &AdapterRegistryObservation,
    tested_capabilities: Option<&BTreeMap<String, CapabilitySupport>>,
    diagnostics: &mut Vec<ContextDiagnostic>,
) -> AdapterSnapshot {
    let registry_present = registry.adapter_ids.contains(&declaration.id);
    let registry_validation = if registry_present {
        Validation {
            state: ValidationState::Match,
            reason: None,
        }
    } else if declaration.maturity == AdapterMaturity::Stub {
        Validation {
            state: ValidationState::DeclaredOnly,
            reason: Some("stub adapters may be absent from the executable registry".to_string()),
        }
    } else {
        diagnostics.push(adapter_diagnostic(
            "adapter.registry_missing",
            DiagnosticSeverity::Error,
            format!(
                "declared adapter {:?} is missing from adapter_registry.rs",
                declaration.id
            ),
        ));
        Validation {
            state: ValidationState::Mismatch,
            reason: Some("declared non-stub adapter is absent from registry".to_string()),
        }
    };

    let observed_platforms = registry
        .platforms
        .get(&declaration.id)
        .map(|platform| vec![platform.clone()]);
    let platform_validation = match &observed_platforms {
        Some(observed) if observed == &declaration.platforms => Validation {
            state: ValidationState::Match,
            reason: None,
        },
        Some(observed) => {
            diagnostics.push(adapter_diagnostic(
                "adapter.platform_mismatch",
                DiagnosticSeverity::Error,
                format!(
                    "adapter {:?} declares platforms {:?} but registry reports {:?}",
                    declaration.id, declaration.platforms, observed
                ),
            ));
            Validation {
                state: ValidationState::Mismatch,
                reason: Some("manifest and registry platforms differ".to_string()),
            }
        }
        None => Validation {
            state: ValidationState::DeclaredOnly,
            reason: Some("adapter has no registry platform observation".to_string()),
        },
    };

    let mut observed_roles = registry
        .default_roles
        .iter()
        .filter_map(|(role, adapter)| (adapter == &declaration.id).then_some(role.clone()))
        .collect::<Vec<_>>();
    observed_roles.sort();
    let mut declared_roles = declaration.default_roles.clone();
    declared_roles.sort();
    let roles_match = observed_roles == declared_roles;
    if !roles_match {
        diagnostics.push(adapter_diagnostic(
            "adapter.default_mismatch",
            DiagnosticSeverity::Error,
            format!(
                "adapter {:?} declares default roles {:?} but registry reports {:?}",
                declaration.id, declared_roles, observed_roles
            ),
        ));
    }

    let capabilities = declaration
        .capabilities
        .iter()
        .map(|(name, declared)| {
            let observed = tested_capabilities.and_then(|tested| tested.get(name)).cloned();
            let validation = match &observed {
                Some(observed) if *observed == *declared => Validation {
                    state: ValidationState::Match,
                    reason: None,
                },
                Some(observed) => {
                    diagnostics.push(adapter_diagnostic(
                        "adapter.capability_mismatch",
                        DiagnosticSeverity::Error,
                        format!(
                            "adapter {:?} capability {name:?} declares {declared:?} but current evidence reports {observed:?}",
                            declaration.id
                        ),
                    ));
                    Validation {
                        state: ValidationState::Mismatch,
                        reason: Some("declaration and current profile evidence differ".to_string()),
                    }
                }
                None => {
                    if matches!(declared, CapabilitySupport::Yes | CapabilitySupport::Limited) {
                        diagnostics.push(adapter_diagnostic(
                            "adapter.capability_unvalidated",
                            DiagnosticSeverity::Warning,
                            format!(
                                "adapter {:?} capability {name:?} has no current profile evidence",
                                declaration.id
                            ),
                        ));
                    }
                    Validation {
                        state: ValidationState::DeclaredOnly,
                        reason: Some("no current profile evidence".to_string()),
                    }
                }
            };
            (
                name.clone(),
                Fact::new(
                    Some(declared.clone()),
                    observed,
                    validation,
                    vec![EvidenceId::from("manifest:adapter"), EvidenceId::from("profile:test")],
                ),
            )
        })
        .collect();

    AdapterSnapshot {
        id: declaration.id.clone(),
        maturity: Fact::new(
            Some(declaration.maturity.clone()),
            None,
            Validation {
                state: ValidationState::DeclaredOnly,
                reason: Some("maturity is declared policy".to_string()),
            },
            vec![EvidenceId::from("manifest:adapter")],
        ),
        platforms: Fact::new(
            Some(declaration.platforms.clone()),
            observed_platforms,
            platform_validation,
            vec![
                EvidenceId::from("manifest:adapter"),
                EvidenceId::from("source:adapter_registry"),
            ],
        ),
        default_roles: Fact::new(
            Some(declaration.default_roles.clone()),
            Some(observed_roles),
            Validation {
                state: if roles_match {
                    ValidationState::Match
                } else {
                    ValidationState::Mismatch
                },
                reason: (!roles_match)
                    .then(|| "manifest and registry default roles differ".to_string()),
            },
            vec![
                EvidenceId::from("manifest:adapter"),
                EvidenceId::from("source:adapter_registry"),
            ],
        ),
        registry_presence: Fact::new(
            Some(true),
            Some(registry_present),
            registry_validation,
            vec![
                EvidenceId::from("manifest:adapter"),
                EvidenceId::from("source:adapter_registry"),
            ],
        ),
        capabilities,
    }
}

fn adapter_diagnostic(
    code: &str,
    severity: DiagnosticSeverity,
    message: String,
) -> ContextDiagnostic {
    ContextDiagnostic {
        code: code.to_string(),
        severity,
        message,
        profile_id: None,
        evidence_ids: vec![
            EvidenceId::from("manifest:adapter"),
            EvidenceId::from("source:adapter_registry"),
        ],
    }
}

pub(super) fn collect_repository_identity(
    runner: &impl CommandRunner,
    root: &Path,
) -> Result<RepositoryIdentity> {
    let root = root
        .canonicalize()
        .with_context(|| format!("canonicalize repository root {}", root.display()))?;
    let commit_output = run_git(runner, &root, &["rev-parse", "HEAD"])?;
    let branch_output = run_git(runner, &root, &["branch", "--show-current"])?;
    let status_output = run_git(
        runner,
        &root,
        &["status", "--porcelain=v2", "-z", "--untracked-files=all"],
    )?;

    let commit = output_text(&commit_output.stdout, "git rev-parse")?
        .trim()
        .to_string();
    let branch = match output_text(&branch_output.stdout, "git branch")?.trim() {
        "" => "HEAD".to_string(),
        branch => branch.to_string(),
    };
    let changed_paths = parse_porcelain_v2_paths(&root, &status_output.stdout)?;
    let worktree_fingerprint = fingerprint_worktree(&root, &status_output.stdout, &changed_paths)?;

    Ok(RepositoryIdentity {
        commit,
        branch,
        dirty: !changed_paths.is_empty(),
        changed_paths,
        worktree_fingerprint,
        evidence_ids: vec![
            EvidenceId::from("git:branch"),
            EvidenceId::from("git:head"),
            EvidenceId::from("git:status"),
        ],
    })
}

fn run_git(runner: &impl CommandRunner, root: &Path, args: &[&str]) -> Result<CommandOutput> {
    let output = runner.run(&CommandSpec {
        program: "git".to_string(),
        args: args.iter().map(ToString::to_string).collect(),
        current_dir: root.to_path_buf(),
    })?;
    if output.exit_code != Some(0) {
        bail!(
            "git {} failed with status {:?}: {}",
            args.join(" "),
            output.exit_code,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output)
}

fn output_text<'a>(bytes: &'a [u8], command: &str) -> Result<&'a str> {
    std::str::from_utf8(bytes).with_context(|| format!("{command} output is not UTF-8"))
}

fn parse_porcelain_v2_paths(root: &Path, output: &[u8]) -> Result<Vec<RepositoryPath>> {
    let mut records = output
        .split(|byte| *byte == 0)
        .filter(|record| !record.is_empty());
    let mut paths = Vec::new();
    while let Some(record) = records.next() {
        let (field_count, consumes_original) = match record.first() {
            Some(b'1') => (9, false),
            Some(b'2') => (10, true),
            Some(b'u') => (11, false),
            Some(b'?') => (2, false),
            Some(prefix) => bail!(
                "unsupported porcelain-v2 record type: {}",
                char::from(*prefix)
            ),
            None => continue,
        };
        let path = record
            .splitn(field_count, |byte| *byte == b' ')
            .nth(field_count - 1)
            .context("porcelain-v2 record is missing a path")?;
        let path = std::str::from_utf8(path).context("Git changed path is not UTF-8")?;
        paths.push(RepositoryPath::from_root(root, &root.join(path))?);
        if consumes_original {
            records
                .next()
                .context("porcelain-v2 rename record is missing its original path")?;
        }
    }
    paths.sort();
    paths.dedup();
    Ok(paths)
}

fn fingerprint_worktree(
    root: &Path,
    status: &[u8],
    changed_paths: &[RepositoryPath],
) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(b"git-status-porcelain-v2\0");
    hasher.update(status);
    for path in changed_paths {
        hasher.update(path.as_str().as_bytes());
        hasher.update(b"\0");
        let absolute = root.join(path.as_str());
        match std::fs::symlink_metadata(&absolute) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                hasher.update(b"symlink\0");
                let target = std::fs::read_link(&absolute)
                    .with_context(|| format!("read symlink {}", absolute.display()))?;
                hasher.update(target.to_string_lossy().as_bytes());
            }
            Ok(metadata) if metadata.is_file() => {
                let canonical = absolute
                    .canonicalize()
                    .with_context(|| format!("canonicalize changed file {}", absolute.display()))?;
                if !canonical.starts_with(root) {
                    bail!(
                        "changed file resolves outside repository: {}",
                        absolute.display()
                    );
                }
                hasher.update(b"file\0");
                hasher.update(
                    std::fs::read(&canonical)
                        .with_context(|| format!("read changed file {}", absolute.display()))?,
                );
            }
            Ok(_) => hasher.update(b"other\0"),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                hasher.update(b"missing\0");
            }
            Err(error) => {
                return Err(error)
                    .with_context(|| format!("inspect changed path {}", absolute.display()));
            }
        }
        hasher.update(b"\0");
    }
    Ok(hex::encode(hasher.finalize()))
}

pub(super) fn collect_workspace(
    runner: &impl CommandRunner,
    repository: &impl RepositoryReader,
    root: &Path,
) -> Result<WorkspaceSnapshot> {
    let canonical_root = root
        .canonicalize()
        .with_context(|| format!("canonicalize workspace root {}", root.display()))?;
    let output = runner.run(&CommandSpec {
        program: "cargo".to_string(),
        args: vec![
            "metadata".to_string(),
            "--format-version".to_string(),
            "1".to_string(),
        ],
        current_dir: root.to_path_buf(),
    })?;
    if output.exit_code != Some(0) {
        bail!(
            "cargo metadata failed with status {:?}: {}",
            output.exit_code,
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let stdout =
        std::str::from_utf8(&output.stdout).context("cargo metadata output is not UTF-8")?;
    let metadata = MetadataCommand::parse(stdout).context("parse cargo metadata output")?;
    let workspace_members = metadata
        .workspace_members
        .iter()
        .map(|id| id.repr.as_str())
        .collect::<BTreeSet<_>>();
    let workspace_packages = metadata
        .packages
        .iter()
        .filter(|package| workspace_members.contains(package.id.repr.as_str()))
        .collect::<Vec<_>>();
    let version = common_value(
        workspace_packages
            .iter()
            .map(|package| Some(package.version.to_string())),
    );
    let edition = common_value(
        workspace_packages
            .iter()
            .map(|package| Some(package.edition.to_string())),
    );
    let msrv = common_value(
        workspace_packages
            .iter()
            .map(|package| package.rust_version.as_ref().map(ToString::to_string)),
    );
    let tracked_files = repository.tracked_files(&canonical_root)?;
    let package_roots = workspace_packages
        .iter()
        .map(|package| {
            package
                .manifest_path
                .parent()
                .map(|path| path.as_std_path().to_path_buf())
                .context("workspace package manifest has no parent")
        })
        .collect::<Result<Vec<_>>>()?;
    let mut packages = workspace_packages
        .into_iter()
        .map(|package| {
            package_snapshot(
                &canonical_root,
                package,
                repository,
                &tracked_files,
                &package_roots,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    packages.sort_by(|left, right| left.package_id.cmp(&right.package_id));

    Ok(WorkspaceSnapshot {
        version: observed_workspace_fact(version),
        edition: observed_workspace_fact(edition),
        msrv: observed_workspace_fact(msrv),
        packages,
    })
}

fn package_snapshot(
    root: &Path,
    package: &cargo_metadata::Package,
    repository: &impl RepositoryReader,
    tracked_files: &[RepositoryPath],
    package_roots: &[PathBuf],
) -> Result<PackageSnapshot> {
    let mut targets = package
        .targets
        .iter()
        .map(|target| {
            let mut kinds = target
                .kind
                .iter()
                .map(|kind| {
                    serde_json::to_value(kind)
                        .context("serialize Cargo target kind")?
                        .as_str()
                        .map(str::to_string)
                        .context("Cargo target kind was not a string")
                })
                .collect::<Result<Vec<_>>>()?;
            kinds.sort();
            let mut required_features = target.required_features.clone();
            required_features.sort();
            required_features.dedup();
            Ok(TargetSnapshot {
                name: target.name.clone(),
                kinds,
                source_path: RepositoryPath::from_root(root, target.src_path.as_std_path())?,
                required_features,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    targets.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.source_path.cmp(&right.source_path))
    });

    let mut dependencies = package
        .dependencies
        .iter()
        .map(|dependency| {
            let kind = match dependency.kind {
                CargoDependencyKind::Normal => DependencyKind::Normal,
                CargoDependencyKind::Build => DependencyKind::Build,
                CargoDependencyKind::Development => DependencyKind::Dev,
                CargoDependencyKind::Unknown => {
                    bail!("unknown Cargo dependency kind for {}", dependency.name)
                }
            };
            Ok(DependencySnapshot {
                package_name: dependency.name.clone(),
                rename: dependency.rename.clone(),
                kind,
                target_predicate: dependency.target.as_ref().map(ToString::to_string),
                optional: dependency.optional,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    dependencies.sort_by(|left, right| {
        left.package_name
            .cmp(&right.package_name)
            .then_with(|| left.rename.cmp(&right.rename))
            .then_with(|| dependency_kind_rank(&left.kind).cmp(&dependency_kind_rank(&right.kind)))
            .then_with(|| left.target_predicate.cmp(&right.target_predicate))
    });

    Ok(PackageSnapshot {
        package_id: package.id.repr.clone(),
        name: package.name.to_string(),
        manifest_path: RepositoryPath::from_root(root, package.manifest_path.as_std_path())?,
        targets,
        features: package.features.keys().cloned().collect(),
        dependencies,
        metrics: collect_source_metrics(root, package, repository, tracked_files, package_roots)?,
    })
}

fn collect_source_metrics(
    root: &Path,
    package: &cargo_metadata::Package,
    repository: &impl RepositoryReader,
    tracked_files: &[RepositoryPath],
    package_roots: &[PathBuf],
) -> Result<SourceMetrics> {
    let package_root = package
        .manifest_path
        .parent()
        .map(cargo_metadata::camino::Utf8Path::as_std_path)
        .context("package manifest has no parent")?;
    let mut scope_roots = ["src", "tests", "examples", "benches"]
        .into_iter()
        .map(|directory| package_root.join(directory))
        .collect::<BTreeSet<_>>();
    for target in &package.targets {
        let source = target.src_path.as_std_path();
        let in_conventional_root = ["src", "tests", "examples", "benches"]
            .into_iter()
            .any(|directory| source.starts_with(package_root.join(directory)));
        if !in_conventional_root {
            let scope = source
                .parent()
                .filter(|parent| *parent != package_root)
                .map_or_else(|| source.to_path_buf(), Path::to_path_buf);
            scope_roots.insert(scope);
        }
    }

    let mut metrics = SourceMetrics {
        rust_files: 0,
        physical_lines: 0,
        non_empty_lines: 0,
        included_roots: Vec::new(),
        includes_generated: false,
    };
    let mut included_roots = BTreeSet::new();
    for tracked in tracked_files {
        let path = root.join(tracked.as_str());
        if path.extension().and_then(|extension| extension.to_str()) != Some("rs")
            || path.strip_prefix(package_root).is_ok_and(|relative| {
                relative
                    .components()
                    .any(|component| component.as_os_str() == "target")
            })
            || owning_package_root(&path, package_roots) != Some(package_root)
        {
            continue;
        }
        let Some(scope_root) = scope_roots.iter().find(|scope_root| {
            if scope_root
                .extension()
                .and_then(|extension| extension.to_str())
                == Some("rs")
            {
                path == **scope_root
            } else {
                path.starts_with(scope_root)
            }
        }) else {
            continue;
        };

        let file = repository.file_metrics(&path)?;
        metrics.rust_files += 1;
        metrics.physical_lines += file.physical_lines;
        metrics.non_empty_lines += file.non_empty_lines;
        included_roots.insert(RepositoryPath::from_root(root, scope_root)?);
    }
    metrics.included_roots = included_roots.into_iter().collect();
    Ok(metrics)
}

fn owning_package_root<'a>(path: &Path, package_roots: &'a [PathBuf]) -> Option<&'a Path> {
    package_roots
        .iter()
        .filter(|package_root| path.starts_with(package_root))
        .max_by_key(|package_root| package_root.components().count())
        .map(PathBuf::as_path)
}

const fn dependency_kind_rank(kind: &DependencyKind) -> u8 {
    match kind {
        DependencyKind::Normal => 0,
        DependencyKind::Build => 1,
        DependencyKind::Dev => 2,
    }
}

fn common_value(values: impl Iterator<Item = Option<String>>) -> Option<String> {
    let mut values = values;
    let first = values.next()??;
    values
        .all(|value| value.as_ref() == Some(&first))
        .then_some(first)
}

fn observed_workspace_fact(value: Option<String>) -> Fact<String> {
    let (state, reason) = if value.is_some() {
        (ValidationState::ObservedOnly, None)
    } else {
        (
            ValidationState::Unavailable,
            Some("workspace packages do not share one value".to_string()),
        )
    };
    Fact::new(None, value, Validation { state, reason }, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::model::ProfileStatus;
    use std::process::Command;

    #[test]
    fn system_ports_preserve_status_and_tracked_paths() {
        let temp = tempfile::tempdir().expect("temporary repository should be created");
        let root = temp.path();
        std::fs::create_dir(root.join("nested"))
            .expect("nested fixture directory should be created");
        std::fs::write(root.join("alpha.txt"), b"alpha\n\nbeta\n")
            .expect("metrics fixture should be written");
        std::fs::write(root.join("nested/data.txt"), b"tracked\n")
            .expect("tracked fixture should be written");

        for args in [
            vec!["init", "--quiet"],
            vec!["add", "--", "alpha.txt", "nested/data.txt"],
        ] {
            let status = Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .expect("git fixture command should run");
            assert!(status.success(), "git fixture command should succeed");
        }

        #[cfg(unix)]
        let command = CommandSpec {
            program: "/bin/sh".to_string(),
            args: vec![
                "-c".to_string(),
                "echo stdout; echo diagnostic >&2; exit 7".to_string(),
            ],
            current_dir: root.to_path_buf(),
        };
        #[cfg(windows)]
        let command = CommandSpec {
            program: "cmd".to_string(),
            args: vec![
                "/C".to_string(),
                "<nul set /p =stdout&echo.&<nul set /p =diagnostic 1>&2&echo. 1>&2&exit /b 7"
                    .to_string(),
            ],
            current_dir: root.to_path_buf(),
        };

        let output = SystemCommandRunner
            .run(&command)
            .expect("system command should run");
        assert_eq!(output.exit_code, Some(7));
        assert_eq!(output.stdout, b"stdout\n");
        assert_eq!(output.stderr, b"diagnostic\n");

        let repository = SystemRepositoryReader;
        let tracked = repository
            .tracked_files(root)
            .expect("tracked files should be listed");
        let tracked: Vec<&str> = tracked.iter().map(RepositoryPath::as_str).collect();
        assert_eq!(tracked, vec!["alpha.txt", "nested/data.txt"]);
        assert_eq!(
            repository
                .read_utf8(&root.join("alpha.txt"))
                .expect("fixture should be readable"),
            "alpha\n\nbeta\n"
        );

        let metrics = repository
            .file_metrics(&root.join("alpha.txt"))
            .expect("file metrics should be collected");
        assert_eq!(metrics.physical_lines, 3);
        assert_eq!(metrics.non_empty_lines, 2);
        assert_eq!(
            metrics.content_sha256,
            "0e937c34625ef864d694878f01fd96318893d29014a51536006dacd150d809ec"
        );
    }
    #[test]
    fn cargo_metadata_preserves_renames_targets_and_dependency_kinds() {
        let temp = tempfile::tempdir().expect("temporary workspace should be created");
        let root = temp.path();
        for directory in ["renamed/source", "helper/src", "dev-helper/src"] {
            std::fs::create_dir_all(root.join(directory))
                .expect("fixture directory should be created");
        }
        std::fs::write(
            root.join("Cargo.toml"),
            r#"[workspace]
resolver = "3"
members = ["renamed", "helper", "dev-helper"]

[workspace.package]
version = "1.2.3"
edition = "2024"
rust-version = "1.85"
"#,
        )
        .expect("workspace manifest should be written");
        std::fs::write(
            root.join("renamed/Cargo.toml"),
            r#"[package]
name = "published-name"
version.workspace = true
edition.workspace = true
rust-version.workspace = true

[lib]
name = "renamed_lib"
path = "source/custom.rs"

[[bin]]
name = "gated-tool"
path = "source/tool.rs"
required-features = ["tooling"]

[features]
default = []
tooling = []

[dependencies]
helper_alias = { package = "helper-package", path = "../helper", optional = true }

[dev-dependencies]
dev_alias = { package = "dev-helper-package", path = "../dev-helper" }
"#,
        )
        .expect("renamed package manifest should be written");
        std::fs::write(
            root.join("renamed/source/custom.rs"),
            "pub fn library() {}\n",
        )
        .expect("custom library source should be written");
        std::fs::write(root.join("renamed/source/tool.rs"), "fn main() {}\n")
            .expect("custom binary source should be written");

        for (directory, name) in [
            ("helper", "helper-package"),
            ("dev-helper", "dev-helper-package"),
        ] {
            std::fs::write(
                root.join(directory).join("Cargo.toml"),
                format!(
                    r#"[package]
name = "{name}"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
"#
                ),
            )
            .expect("dependency manifest should be written");
            std::fs::write(
                root.join(directory).join("src/lib.rs"),
                "pub fn helper() {}\n",
            )
            .expect("dependency source should be written");
        }

        for args in [vec!["init", "--quiet"], vec!["add", "--all"]] {
            let status = Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .expect("git fixture command should run");
            assert!(status.success(), "git fixture command should succeed");
        }

        let workspace = collect_workspace(&SystemCommandRunner, &SystemRepositoryReader, root)
            .expect("workspace metadata should be collected");
        let package = workspace
            .packages
            .iter()
            .find(|package| package.name == "published-name")
            .expect("renamed package should be present");

        assert!(package.package_id.ends_with("#published-name@1.2.3"));
        assert_eq!(package.manifest_path.as_str(), "renamed/Cargo.toml");
        assert_eq!(package.features, ["default", "helper_alias", "tooling"]);

        let library = package
            .targets
            .iter()
            .find(|target| target.name == "renamed_lib")
            .expect("custom library target should be present");
        assert_eq!(library.kinds, ["lib"]);
        assert_eq!(library.source_path.as_str(), "renamed/source/custom.rs");

        let binary = package
            .targets
            .iter()
            .find(|target| target.name == "gated-tool")
            .expect("required-features target should be present");
        assert_eq!(binary.kinds, ["bin"]);
        assert_eq!(binary.source_path.as_str(), "renamed/source/tool.rs");
        assert_eq!(binary.required_features, ["tooling"]);

        let normal = package
            .dependencies
            .iter()
            .find(|dependency| dependency.package_name == "helper-package")
            .expect("renamed normal dependency should be present");
        assert_eq!(normal.rename.as_deref(), Some("helper_alias"));
        assert_eq!(normal.kind, DependencyKind::Normal);
        assert!(normal.optional);

        let dev = package
            .dependencies
            .iter()
            .find(|dependency| dependency.package_name == "dev-helper-package")
            .expect("renamed dev dependency should be present");
        assert_eq!(dev.rename.as_deref(), Some("dev_alias"));
        assert_eq!(dev.kind, DependencyKind::Dev);
        assert!(!dev.optional);
    }
    #[test]
    fn source_metrics_state_scope_and_line_semantics() {
        let temp = tempfile::tempdir().expect("temporary workspace should be created");
        let root = temp.path();
        for directory in [
            "metrics/src",
            "metrics/tests",
            "metrics/examples",
            "metrics/benches",
            "metrics/target",
        ] {
            std::fs::create_dir_all(root.join(directory))
                .expect("fixture directory should be created");
        }
        std::fs::write(
            root.join("Cargo.toml"),
            r#"[workspace]
resolver = "3"
members = ["metrics"]
"#,
        )
        .expect("workspace manifest should be written");
        std::fs::write(
            root.join("metrics/Cargo.toml"),
            r#"[package]
name = "metrics-package"
version = "0.1.0"
edition = "2024"
build = "build.rs"
"#,
        )
        .expect("package manifest should be written");
        for (path, content) in [
            ("metrics/src/lib.rs", "pub fn value() {}\n\n// comment\n"),
            ("metrics/src/empty.rs", ""),
            ("metrics/tests/integration.rs", "\n#[test]\nfn works() {}\n"),
            ("metrics/examples/demo.rs", "fn main() {}\n"),
            ("metrics/benches/bench.rs", "fn main() {\n\n}\n"),
            ("metrics/build.rs", "fn main() {}\n"),
        ] {
            std::fs::write(root.join(path), content)
                .expect("tracked Rust fixture should be written");
        }
        std::fs::write(root.join("metrics/target/generated.rs"), "generated\n")
            .expect("target fixture should be written");
        std::fs::write(root.join("metrics/src/generated.rs"), "untracked\n")
            .expect("untracked source fixture should be written");

        for args in [
            vec!["init", "--quiet"],
            vec![
                "add",
                "--",
                "Cargo.toml",
                "metrics/Cargo.toml",
                "metrics/src/lib.rs",
                "metrics/src/empty.rs",
                "metrics/tests/integration.rs",
                "metrics/examples/demo.rs",
                "metrics/benches/bench.rs",
                "metrics/build.rs",
                "metrics/target/generated.rs",
            ],
        ] {
            let status = Command::new("git")
                .args(args)
                .current_dir(root)
                .status()
                .expect("git fixture command should run");
            assert!(status.success(), "git fixture command should succeed");
        }

        let workspace = collect_workspace(&SystemCommandRunner, &SystemRepositoryReader, root)
            .expect("workspace metrics should be collected");
        let metrics = &workspace
            .packages
            .iter()
            .find(|package| package.name == "metrics-package")
            .expect("fixture package should be present")
            .metrics;
        assert_eq!(metrics.rust_files, 6);
        assert_eq!(metrics.physical_lines, 11);
        assert_eq!(metrics.non_empty_lines, 8);
        assert_eq!(
            metrics
                .included_roots
                .iter()
                .map(RepositoryPath::as_str)
                .collect::<Vec<_>>(),
            [
                "metrics/benches",
                "metrics/build.rs",
                "metrics/examples",
                "metrics/src",
                "metrics/tests",
            ]
        );
        assert!(!metrics.includes_generated);
    }
    #[test]
    fn repository_identity_distinguishes_dirty_worktrees() {
        let temp = tempfile::tempdir().expect("temporary repository root should be created");
        let repository = temp.path().join("repository");
        let worktree_a = temp.path().join("worktree-a");
        let worktree_b = temp.path().join("worktree-b");
        std::fs::create_dir(&repository).expect("repository directory should be created");
        std::fs::write(repository.join("tracked.txt"), "base\n")
            .expect("tracked fixture should be written");
        std::fs::write(repository.join(".gitignore"), "ignored.txt\n")
            .expect("ignore fixture should be written");

        let run_git = |current_dir: &Path, args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(current_dir)
                .status()
                .expect("git fixture command should run");
            assert!(
                status.success(),
                "git fixture command should succeed: {args:?}"
            );
        };
        run_git(&repository, &["init", "--quiet"]);
        run_git(&repository, &["add", "--all"]);
        run_git(
            &repository,
            &[
                "-c",
                "user.name=Context Test",
                "-c",
                "user.email=context@example.invalid",
                "commit",
                "--quiet",
                "-m",
                "fixture",
            ],
        );
        for worktree in [&worktree_a, &worktree_b] {
            let status = Command::new("git")
                .args(["worktree", "add", "--quiet", "--detach"])
                .arg(worktree)
                .arg("HEAD")
                .current_dir(&repository)
                .status()
                .expect("git worktree command should run");
            assert!(status.success(), "git worktree command should succeed");
        }

        std::fs::write(worktree_a.join("tracked.txt"), "changed\n")
            .expect("tracked worktree change should be written");
        std::fs::write(worktree_b.join("untracked.txt"), "new\n")
            .expect("untracked worktree change should be written");
        std::fs::write(worktree_b.join("ignored.txt"), "ignored-one\n")
            .expect("ignored worktree file should be written");

        let identity_a = collect_repository_identity(&SystemCommandRunner, &worktree_a)
            .expect("first worktree identity should be collected");
        let identity_b = collect_repository_identity(&SystemCommandRunner, &worktree_b)
            .expect("second worktree identity should be collected");
        assert_eq!(identity_a.commit, identity_b.commit);
        assert_eq!(identity_a.branch, identity_b.branch);
        assert_eq!(
            identity_a
                .changed_paths
                .iter()
                .map(RepositoryPath::as_str)
                .collect::<Vec<_>>(),
            ["tracked.txt"]
        );
        assert_eq!(
            identity_b
                .changed_paths
                .iter()
                .map(RepositoryPath::as_str)
                .collect::<Vec<_>>(),
            ["untracked.txt"]
        );
        assert!(identity_a.dirty);
        assert!(identity_b.dirty);
        assert_ne!(
            identity_a.worktree_fingerprint,
            identity_b.worktree_fingerprint
        );
        assert_eq!(identity_a.worktree_fingerprint.len(), 64);
        assert_eq!(
            identity_a.evidence_ids,
            [
                EvidenceId::from("git:branch"),
                EvidenceId::from("git:head"),
                EvidenceId::from("git:status"),
            ]
        );

        std::fs::write(worktree_b.join("ignored.txt"), "ignored-two\n")
            .expect("ignored worktree file should be updated");
        let identity_b_after_ignored_change =
            collect_repository_identity(&SystemCommandRunner, &worktree_b)
                .expect("updated worktree identity should be collected");
        assert_eq!(
            identity_b.worktree_fingerprint,
            identity_b_after_ignored_change.worktree_fingerprint
        );
    }
    #[test]
    fn profile_evidence_requires_an_exact_cache_key() {
        use super::output::{
            ProfileEvidenceCacheKey, profile_evidence_cache_key, read_profile_evidence,
        };

        let baseline = ProfileEvidenceCacheKey {
            commit: "abc123".to_string(),
            worktree_fingerprint: "dirty-a".to_string(),
            manifest_sha256: "manifest-a".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            features: vec!["zeta".to_string(), "alpha".to_string()],
            no_default_features: false,
            cargo_version: "cargo 1.85.0".to_string(),
            rustc_version: "rustc 1.85.0".to_string(),
            nextest_version: "cargo-nextest 0.9.100".to_string(),
        };
        let mut reordered = baseline.clone();
        reordered.features.reverse();
        assert_eq!(
            profile_evidence_cache_key(&baseline),
            profile_evidence_cache_key(&reordered)
        );

        let evidence_dir = tempfile::tempdir().expect("evidence directory should be created");
        let artifact = |key: &ProfileEvidenceCacheKey| {
            serde_json::json!({
                "schema_version": 1,
                "profile_id": "native-linux",
                "cache_key": key,
                "status": "validated",
                "executable_tests": [{
                    "stable_id": "pkg::bin::works",
                    "package_id": "pkg",
                    "binary_id": "bin",
                    "test_name": "works",
                    "ignored": false
                }],
                "unavailable_reason": null
            })
        };
        std::fs::write(
            evidence_dir.path().join("00-current.json"),
            serde_json::to_vec(&artifact(&baseline)).expect("current artifact should serialize"),
        )
        .expect("current artifact should be written");

        let mut mismatches = Vec::new();
        let mut changed = baseline.clone();
        changed.commit = "different".to_string();
        mismatches.push(changed);
        let mut changed = baseline.clone();
        changed.worktree_fingerprint = "different".to_string();
        mismatches.push(changed);
        let mut changed = baseline.clone();
        changed.manifest_sha256 = "different".to_string();
        mismatches.push(changed);
        let mut changed = baseline.clone();
        changed.target = "aarch64-apple-darwin".to_string();
        mismatches.push(changed);
        let mut changed = baseline.clone();
        changed.features.push("different".to_string());
        mismatches.push(changed);
        let mut changed = baseline.clone();
        changed.no_default_features = true;
        mismatches.push(changed);
        let mut changed = baseline.clone();
        changed.cargo_version = "different".to_string();
        mismatches.push(changed);
        let mut changed = baseline.clone();
        changed.rustc_version = "different".to_string();
        mismatches.push(changed);
        let mut changed = baseline.clone();
        changed.nextest_version = "different".to_string();
        mismatches.push(changed);

        for (index, mismatch) in mismatches.iter().enumerate() {
            std::fs::write(
                evidence_dir
                    .path()
                    .join(format!("{:02}-stale.json", index + 1)),
                serde_json::to_vec(&artifact(mismatch)).expect("stale artifact should serialize"),
            )
            .expect("stale artifact should be written");
        }

        let expected =
            std::collections::BTreeMap::from([("native-linux".to_string(), baseline.clone())]);
        let imported = read_profile_evidence(evidence_dir.path(), &expected)
            .expect("well-formed evidence should import");
        assert_eq!(imported.len(), 10);
        assert_eq!(
            imported
                .iter()
                .filter(|record| record.status == ProfileStatus::Validated)
                .count(),
            1
        );
        assert_eq!(
            imported
                .iter()
                .filter(|record| record.status == ProfileStatus::Stale)
                .count(),
            9
        );
        assert_eq!(
            imported
                .iter()
                .flat_map(|record| &record.executable_tests)
                .count(),
            1
        );

        let malformed_dir = tempfile::tempdir().expect("malformed directory should be created");
        std::fs::write(malformed_dir.path().join("broken.json"), b"{}")
            .expect("malformed artifact should be written");
        assert!(read_profile_evidence(malformed_dir.path(), &expected).is_err());
    }
}
