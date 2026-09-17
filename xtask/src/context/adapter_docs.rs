use super::collect::output::sync_adapter_matrix;
use super::manifest::{load_manifest, validate_manifest};
use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

const MATRIX_PATH: &str = "docs/core/FEATURE_MATRIX.mbx.md";

pub(super) fn sync(root: &Path) -> Result<()> {
    let (path, original, updated) = prepare(root)?;
    if updated != original {
        super::collect::output::atomic_write(&path, updated.as_bytes())?;
    }
    Ok(())
}

pub(super) fn check_drift(root: &Path) -> Result<()> {
    let (_, original, updated) = prepare(root)?;
    if normalize(&original) != normalize(&updated) {
        bail!("generated adapter documentation differs from xtask/context.toml");
    }
    Ok(())
}

fn prepare(root: &Path) -> Result<(PathBuf, String, String)> {
    let manifest = load_manifest(root)?;
    validate_manifest(&manifest)?;
    let path = root.join(MATRIX_PATH);
    let original =
        std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
    let updated = sync_adapter_matrix(&original, &manifest)?;
    Ok((path, original, updated))
}

fn normalize(document: &str) -> String {
    document
        .replace("\r\n", "\n")
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = r#"schema_version = 1
[[adapters]]
id = "native"
maturity = "production"
platforms = ["linux"]
default_roles = []
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
"#;

    const DOCUMENT: &str = "before\n<!-- BEGIN GENERATED: adapter-suites -->\nold\n<!-- END GENERATED: adapter-suites -->\nmiddle\n<!-- BEGIN GENERATED: adapter-capabilities -->\nold\n<!-- END GENERATED: adapter-capabilities -->\nafter\n";

    fn fixture(manifest: &str) -> tempfile::TempDir {
        let temp = tempfile::tempdir().expect("temporary docs root should be created");
        std::fs::create_dir_all(temp.path().join("xtask"))
            .expect("xtask directory should be created");
        std::fs::create_dir_all(temp.path().join("docs/core"))
            .expect("docs directory should be created");
        std::fs::write(temp.path().join("xtask/context.toml"), manifest)
            .expect("manifest should be written");
        std::fs::write(temp.path().join(MATRIX_PATH), DOCUMENT)
            .expect("document should be written");
        temp
    }

    #[test]
    fn sync_is_direct_and_idempotent() {
        let temp = fixture(MANIFEST);
        sync(temp.path()).expect("first sync should succeed");
        let first = std::fs::read(temp.path().join(MATRIX_PATH)).expect("matrix should be read");
        sync(temp.path()).expect("second sync should succeed");
        let second = std::fs::read(temp.path().join(MATRIX_PATH)).expect("matrix should be read");
        assert_eq!(first, second);
        check_drift(temp.path()).expect("synchronized matrix should have no drift");
    }

    #[test]
    fn invalid_manifest_does_not_write_document() {
        let temp = fixture(&MANIFEST.replace("schema_version = 1", "schema_version = 2"));
        let before = std::fs::read(temp.path().join(MATRIX_PATH)).expect("matrix should be read");
        assert!(sync(temp.path()).is_err());
        assert_eq!(
            std::fs::read(temp.path().join(MATRIX_PATH)).expect("matrix should be read"),
            before
        );
    }
}
