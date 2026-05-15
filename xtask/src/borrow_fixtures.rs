use anyhow::{Context, Result, bail};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::Command,
};
use tempfile::TempDir;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Expectation {
    Pass,
    Fail,
}

impl Expectation {
    fn label(self) -> &'static str {
        match self {
            Self::Pass => "must-pass",
            Self::Fail => "must-fail",
        }
    }
}

#[derive(Debug)]
struct Fixture {
    path: PathBuf,
    expectation: Expectation,
    expected_diagnostics: Vec<String>,
}

/// Run Rust borrow-reasoning fixtures.
///
/// Fixtures under `xtask/fixtures/borrow/pass` must compile. Fixtures under
/// `xtask/fixtures/borrow/fail` must fail to compile and may declare required
/// diagnostic snippets with one or more leading `// expect: ...` comments.
pub fn run(root: &Path) -> Result<()> {
    let fixture_root = root.join("xtask/fixtures/borrow");
    let fixtures = collect_fixtures(&fixture_root)?;
    if fixtures.is_empty() {
        bail!(
            "no borrow-reasoning fixtures found under {}",
            fixture_root.display()
        );
    }

    let out_dir = TempDir::new().context("create temporary rustc output directory")?;
    let mut failures = Vec::new();
    let compile_count = fixtures
        .iter()
        .filter(|fixture| fixture.expectation == Expectation::Pass)
        .count();
    let reject_count = fixtures.len() - compile_count;

    eprintln!("borrow-reasoning fixtures:");
    for fixture in &fixtures {
        let label = fixture.expectation.label();
        let name = fixture_name(&fixture.path)?;

        match run_fixture(fixture, out_dir.path()) {
            Ok(()) => eprintln!("  ok   [{label}] {name}"),
            Err(err) => {
                eprintln!("  FAILED [{label}] {name}");
                failures.push(format!("{}: {err:#}", fixture.path.display()));
            }
        }
    }

    if !failures.is_empty() {
        bail!(
            "{} borrow-reasoning fixture(s) failed:\n  {}",
            failures.len(),
            failures.join("\n  ")
        );
    }

    eprintln!(
        "borrow-reasoning fixtures passed: {compile_count} must-pass, {reject_count} must-fail"
    );
    Ok(())
}

fn collect_fixtures(fixture_root: &Path) -> Result<Vec<Fixture>> {
    let mut fixtures = Vec::new();

    for (dir_name, expectation) in [("pass", Expectation::Pass), ("fail", Expectation::Fail)] {
        let dir = fixture_root.join(dir_name);
        let mut paths = read_rust_files(&dir)
            .with_context(|| format!("read borrow fixture directory {}", dir.display()))?;
        paths.sort();

        for path in paths {
            let expected_diagnostics = expected_diagnostics(&path)?;
            if expectation == Expectation::Fail && expected_diagnostics.is_empty() {
                bail!(
                    "failing fixture {} must include at least one `// expect: ...` diagnostic snippet",
                    path.display()
                );
            }
            fixtures.push(Fixture {
                path,
                expectation,
                expected_diagnostics,
            });
        }
    }

    Ok(fixtures)
}

fn read_rust_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension() == Some(OsStr::new("rs")) {
            paths.push(path);
        }
    }
    Ok(paths)
}

fn expected_diagnostics(path: &Path) -> Result<Vec<String>> {
    let content = fs::read_to_string(path)
        .with_context(|| format!("read borrow fixture {}", path.display()))?;
    Ok(content
        .lines()
        .map(str::trim)
        .filter_map(|line| line.strip_prefix("// expect:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect())
}

fn run_fixture(fixture: &Fixture, out_dir: &Path) -> Result<()> {
    let output_path = out_dir.join(format!("{}.rmeta", crate_name(&fixture.path)?));
    let output = Command::new("rustc")
        .arg("--edition=2024")
        .arg("--crate-type=bin")
        .arg("--emit=metadata")
        .arg("-o")
        .arg(&output_path)
        .arg(&fixture.path)
        .output()
        .with_context(|| format!("launch rustc for {}", fixture.path.display()))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let diagnostics = format!("{stdout}{stderr}");

    match (fixture.expectation, output.status.success()) {
        (Expectation::Pass, true) => Ok(()),
        (Expectation::Pass, false) => {
            bail!(
                "expected fixture to compile, but rustc failed:\n{}",
                diagnostics.trim()
            )
        }
        (Expectation::Fail, true) => {
            bail!("expected fixture to fail borrow checking, but it compiled")
        }
        (Expectation::Fail, false) => {
            for expected in &fixture.expected_diagnostics {
                if !diagnostics.contains(expected) {
                    bail!(
                        "expected rustc diagnostics to contain `{expected}`, got:\n{}",
                        diagnostics.trim()
                    );
                }
            }
            Ok(())
        }
    }
}

fn crate_name(path: &Path) -> Result<String> {
    let stem = path
        .file_stem()
        .and_then(OsStr::to_str)
        .with_context(|| format!("fixture path has no UTF-8 file stem: {}", path.display()))?;
    let sanitized: String = stem
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    Ok(format!("borrow_fixture_{sanitized}"))
}

fn fixture_name(path: &Path) -> Result<&str> {
    path.file_name()
        .and_then(OsStr::to_str)
        .with_context(|| format!("fixture path has no UTF-8 file name: {}", path.display()))
}
