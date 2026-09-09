use super::model::{FileMetrics, RepositoryPath};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
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
}
