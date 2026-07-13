//! Verifies that all Dockerfile fixtures under `tests/e2e/images/` are
//! parseable by the minibox Dockerfile parser without panicking.
//!
//! This test is platform-independent: it only exercises the parser, not the
//! builder runtime. It acts as a syntax gate — if a fixture is malformed the
//! test will fail with a clear error before any e2e run attempts to use it.

use minibox_core::image::dockerfile::parse;
use std::path::{Path, PathBuf};

/// Locate the workspace root by searching upward from this file's directory
/// for a `Cargo.toml` that contains `[workspace]`.
fn workspace_root() -> PathBuf {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.exists()
            && let Ok(contents) = std::fs::read_to_string(&candidate)
            && contents.contains("[workspace]")
        {
            return dir;
        }
        if !dir.pop() {
            panic!(
                "could not locate workspace root from {}",
                env!("CARGO_MANIFEST_DIR")
            );
        }
    }
}

/// Collect all `Dockerfile` files under `tests/e2e/images/`.
fn collect_fixtures(images_dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if !images_dir.exists() {
        return out;
    }
    for entry in std::fs::read_dir(images_dir)
        .expect("read tests/e2e/images/")
        .flatten()
    {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            let dockerfile = entry.path().join("Dockerfile");
            if dockerfile.exists() {
                out.push(dockerfile);
            }
        }
    }
    out.sort();
    out
}

#[test]
fn miniboxfile_fixtures_are_parseable() {
    let root = workspace_root();
    let images_dir = root.join("tests").join("e2e").join("images");

    let fixtures = collect_fixtures(&images_dir);

    assert!(
        !fixtures.is_empty(),
        "expected at least one Dockerfile under tests/e2e/images/, found none (looked in {})",
        images_dir.display()
    );

    for path in &fixtures {
        let content = std::fs::read_to_string(path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));

        let result = parse(&content);

        assert!(
            result.is_ok(),
            "Dockerfile at {} failed to parse: {}",
            path.display(),
            result.unwrap_err()
        );

        let instructions = result.expect("parse succeeded");
        assert!(
            !instructions.is_empty(),
            "Dockerfile at {} produced zero instructions",
            path.display()
        );
    }
}

#[test]
fn miniboxfile_fixture_alpine_echo_has_from_and_cmd() {
    let root = workspace_root();
    let dockerfile = root
        .join("tests")
        .join("e2e")
        .join("images")
        .join("alpine-echo")
        .join("Dockerfile");

    let content = std::fs::read_to_string(&dockerfile)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", dockerfile.display()));

    let instructions = parse(&content).expect("alpine-echo Dockerfile must parse");

    let has_from = instructions
        .iter()
        .any(|i| matches!(i, minibox_core::image::dockerfile::Instruction::From { .. }));
    assert!(
        has_from,
        "alpine-echo Dockerfile must contain a FROM instruction"
    );

    let has_cmd = instructions
        .iter()
        .any(|i| matches!(i, minibox_core::image::dockerfile::Instruction::Cmd(_)));
    assert!(
        has_cmd,
        "alpine-echo Dockerfile must contain a CMD instruction"
    );
}

#[test]
fn miniboxfile_fixture_hello_world_is_from_scratch() {
    let root = workspace_root();
    let dockerfile = root
        .join("tests")
        .join("e2e")
        .join("images")
        .join("hello-world")
        .join("Dockerfile");

    let content = std::fs::read_to_string(&dockerfile)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", dockerfile.display()));

    let instructions = parse(&content).expect("hello-world Dockerfile must parse");

    let from_scratch = instructions.iter().any(|i| {
        matches!(
            i,
            minibox_core::image::dockerfile::Instruction::From { image, .. }
            if image == "scratch"
        )
    });
    assert!(from_scratch, "hello-world Dockerfile must use FROM scratch");
}
