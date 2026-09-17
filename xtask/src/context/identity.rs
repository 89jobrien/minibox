use super::collect::{
    CommandOutput, CommandRunner, CommandSpec, FingerprintEntry, RepositoryReader,
};
use super::model::{EvidenceId, RepositoryIdentity, RepositoryPath};
use anyhow::{Context, Result, bail};
use sha2::{Digest, Sha256};
use std::path::Path;

pub(super) fn collect_repository_identity(
    runner: &impl CommandRunner,
    repository: &impl RepositoryReader,
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
    let worktree_fingerprint =
        fingerprint_worktree(repository, &root, &status_output.stdout, &changed_paths)?;
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
    repository: &impl RepositoryReader,
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
        match repository.fingerprint_entry(&root.join(path.as_str()), root)? {
            FingerprintEntry::Symlink(target) => {
                hasher.update(b"symlink\0");
                hasher.update(target.to_string_lossy().as_bytes());
            }
            FingerprintEntry::File(bytes) => {
                hasher.update(b"file\0");
                hasher.update(bytes);
            }
            FingerprintEntry::Other => hasher.update(b"other\0"),
            FingerprintEntry::Missing => hasher.update(b"missing\0"),
        }
        hasher.update(b"\0");
    }
    Ok(hex::encode(hasher.finalize()))
}

#[cfg(test)]
mod tests {
    use super::super::model::FileMetrics;
    use super::*;

    struct FailingFingerprintReader;

    impl RepositoryReader for FailingFingerprintReader {
        fn read_utf8(&self, _path: &Path) -> Result<String> {
            unreachable!()
        }

        fn tracked_files(&self, _root: &Path) -> Result<Vec<RepositoryPath>> {
            unreachable!()
        }

        fn file_metrics(&self, _path: &Path) -> Result<FileMetrics> {
            unreachable!()
        }

        fn fingerprint_entry(&self, _path: &Path, _root: &Path) -> Result<FingerprintEntry> {
            bail!("injected fingerprint read failed")
        }
    }

    #[test]
    fn porcelain_parser_covers_rename_delete_and_unmerged_records() {
        let root = tempfile::tempdir().expect("root should be created");
        let output = b"2 R. N... 100644 100644 100644 a b R100 new.rs\0old.rs\01 D. N... 100644 000000 000000 a 0 deleted.rs\0u UU N... 100644 100644 100644 100644 a b c conflict.rs\0";
        let paths =
            parse_porcelain_v2_paths(root.path(), output).expect("identity records should parse");
        assert_eq!(
            paths.iter().map(RepositoryPath::as_str).collect::<Vec<_>>(),
            ["conflict.rs", "deleted.rs", "new.rs"]
        );
    }

    #[test]
    fn porcelain_parser_rejects_malformed_identity_records() {
        let root = tempfile::tempdir().expect("root should be created");
        assert!(parse_porcelain_v2_paths(root.path(), b"2 malformed\0").is_err());
        assert!(parse_porcelain_v2_paths(root.path(), b"x unknown\0").is_err());
    }

    #[test]
    fn fingerprint_propagates_injected_filesystem_failure() {
        let root = tempfile::tempdir().expect("root should be created");
        let changed = RepositoryPath::from_root(root.path(), &root.path().join("changed.rs"))
            .expect("changed path should normalize");
        let error = fingerprint_worktree(
            &FailingFingerprintReader,
            root.path(),
            b"? changed.rs\0",
            &[changed],
        )
        .expect_err("filesystem port failure must propagate");
        assert!(
            error
                .to_string()
                .contains("injected fingerprint read failed")
        );
    }
}
