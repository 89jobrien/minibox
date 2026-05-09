use anyhow::{Result, bail};
use std::{fs, path::Path};

/// CI guard: verify HandlerDependencies construction site count in miniboxd/src/main.rs.
///
/// When a field is added to `HandlerDependencies`, all three adapter suites (native, gke,
/// colima) must be updated together. This check fails if the count deviates from `expected`.
pub fn check_protocol_sites(file: &Path, expected: usize, warn_only: bool) -> Result<()> {
    let content = fs::read_to_string(file)
        .map_err(|e| anyhow::anyhow!("failed to read {}: {e}", file.display()))?;

    let pattern = "HandlerDependencies {";
    let matches: Vec<(usize, &str)> = content
        .lines()
        .enumerate()
        .filter(|(_, line)| line.contains(pattern))
        .collect();

    let count = matches.len();

    println!(
        "check-protocol-sites: found {count} HandlerDependencies construction site(s) in {} (expected {expected})",
        file.display()
    );

    if count != expected {
        let msg = format!(
            "WARN: HandlerDependencies construction site count changed: \
             expected {expected}, got {count}. Update all adapter suites together."
        );
        eprintln!("{msg}");
        for (lineno, line) in &matches {
            eprintln!("  {}:{}: {}", file.display(), lineno + 1, line.trim());
        }
        if warn_only {
            return Ok(());
        }
        bail!(
            "construction site count mismatch ({count} != {expected}); pass --warn-only to suppress"
        );
    }

    println!("OK: construction site count matches expected.");
    for (lineno, line) in &matches {
        println!("  {}:{}: {}", file.display(), lineno + 1, line.trim());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn write_temp(content: &str) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        write!(f, "{content}").unwrap();
        f
    }

    #[test]
    fn count_matches() {
        let f = write_temp(
            "let a = HandlerDependencies { x: 1 };\nlet b = HandlerDependencies { y: 2 };\n",
        );
        assert!(check_protocol_sites(f.path(), 2, false).is_ok());
    }

    #[test]
    fn count_mismatch_hard_fails() {
        let f = write_temp("let a = HandlerDependencies { x: 1 };\n");
        assert!(check_protocol_sites(f.path(), 3, false).is_err());
    }

    #[test]
    fn count_mismatch_warn_only() {
        let f = write_temp("let a = HandlerDependencies { x: 1 };\n");
        assert!(check_protocol_sites(f.path(), 3, true).is_ok());
    }
}
