use super::model::{
    DependencyKind, DependencySnapshot, Fact, FileMetrics, PackageSnapshot, RepositoryPath,
    SourceMetrics, TargetSnapshot, Validation, ValidationState, WorkspaceSnapshot,
};
use anyhow::{Context, Result, bail};
use cargo_metadata::{DependencyKind as CargoDependencyKind, MetadataCommand};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;

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

pub(super) fn collect_workspace(
    runner: &impl CommandRunner,
    _repository: &impl RepositoryReader,
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
    let mut packages = workspace_packages
        .into_iter()
        .map(|package| package_snapshot(&canonical_root, package))
        .collect::<Result<Vec<_>>>()?;
    packages.sort_by(|left, right| left.package_id.cmp(&right.package_id));

    Ok(WorkspaceSnapshot {
        version: observed_workspace_fact(version),
        edition: observed_workspace_fact(edition),
        msrv: observed_workspace_fact(msrv),
        packages,
    })
}

fn package_snapshot(root: &Path, package: &cargo_metadata::Package) -> Result<PackageSnapshot> {
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
        metrics: SourceMetrics {
            rust_files: 0,
            physical_lines: 0,
            non_empty_lines: 0,
            included_roots: Vec::new(),
            includes_generated: false,
        },
    })
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
}
