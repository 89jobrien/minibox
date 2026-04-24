//! CAS (content-addressed storage) overlay helpers.
//!
//! Layout under `~/.minibox/vm/overlay/`:
//!   cas/<sha256>   — file content, named by sha256 of content
//!   refs/<name>    — text file containing a sha256, maps name → CAS object

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

/// Return the default overlay directory: `~/.minibox/vm/overlay/`.
pub fn default_overlay_dir() -> PathBuf {
    crate::vm_image::default_vm_dir().join("overlay")
}

/// SHA-256 hash a file, returning the lowercase hex digest.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 65536];
    loop {
        let n = file
            .read(&mut buf)
            .with_context(|| format!("reading {}", path.display()))?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Add a file to the CAS store under `overlay_dir`.
/// Returns the sha256 hex digest.
/// If `ref_name` is Some, also writes `overlay_dir/refs/<name>` containing the digest.
pub fn cas_add(overlay_dir: &Path, file_path: &Path, ref_name: Option<&str>) -> Result<String> {
    let hash = sha256_file(file_path)?;

    let cas_dir = overlay_dir.join("cas");
    std::fs::create_dir_all(&cas_dir)
        .with_context(|| format!("creating cas dir {}", cas_dir.display()))?;

    let dest = cas_dir.join(&hash);
    std::fs::copy(file_path, &dest)
        .with_context(|| format!("copying {} → {}", file_path.display(), dest.display()))?;

    println!("cas: {}  {}", hash, file_path.display());

    if let Some(name) = ref_name {
        let refs_dir = overlay_dir.join("refs");
        std::fs::create_dir_all(&refs_dir)
            .with_context(|| format!("creating refs dir {}", refs_dir.display()))?;
        let ref_path = refs_dir.join(name);
        std::fs::write(&ref_path, &hash)
            .with_context(|| format!("writing ref {}", ref_path.display()))?;
        println!("ref: {} -> {}", name, hash);
    }

    Ok(hash)
}

/// Check all refs in `overlay_dir/refs/` against their CAS objects.
/// Prints `OK  <name>` or `DRIFT  <name>  expected=<hash>  got=<hash>`.
/// Returns Ok if no drift found, Err if any mismatch.
pub fn cas_check(overlay_dir: &Path) -> Result<()> {
    let refs_dir = overlay_dir.join("refs");
    if !refs_dir.exists() {
        println!("cas-check: no refs dir, nothing to check");
        return Ok(());
    }

    let mut drift = false;

    for entry in
        std::fs::read_dir(&refs_dir).with_context(|| format!("reading {}", refs_dir.display()))?
    {
        let entry = entry.with_context(|| format!("reading entry in {}", refs_dir.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();

        let expected_hash = std::fs::read_to_string(entry.path())
            .with_context(|| format!("reading ref {}", entry.path().display()))?
            .trim()
            .to_string();

        let cas_file = overlay_dir.join("cas").join(&expected_hash);
        if !cas_file.exists() {
            println!("MISSING  {}  expected={}", name, expected_hash);
            drift = true;
            continue;
        }

        let got_hash =
            sha256_file(&cas_file).with_context(|| format!("hashing {}", cas_file.display()))?;

        if got_hash == expected_hash {
            println!("OK  {}", name);
        } else {
            println!(
                "DRIFT  {}  expected={}  got={}",
                name, expected_hash, got_hash
            );
            drift = true;
        }
    }

    if drift {
        anyhow::bail!("cas-check: drift detected");
    }
    Ok(())
}

/// Read `overlay_dir/refs/` and return a vec of `(name, sha256)` pairs, sorted by name.
/// Returns empty vec if refs dir is absent or empty.
pub fn read_refs(overlay_dir: &Path) -> Result<Vec<(String, String)>> {
    let refs_dir = overlay_dir.join("refs");
    if !refs_dir.exists() {
        return Ok(vec![]);
    }

    let mut refs = Vec::new();
    for entry in
        std::fs::read_dir(&refs_dir).with_context(|| format!("reading {}", refs_dir.display()))?
    {
        let entry = entry.with_context(|| format!("reading entry in {}", refs_dir.display()))?;
        if !entry
            .file_type()
            .with_context(|| format!("file_type for {}", entry.path().display()))?
            .is_file()
        {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let hash = std::fs::read_to_string(entry.path())
            .with_context(|| format!("reading ref {}", entry.path().display()))?
            .trim()
            .to_string();
        refs.push((name, hash));
    }
    refs.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(refs)
}

/// Write `/etc/minibox-cas-refs` into `rootfs_dir` from `overlay_dir/refs/`.
/// Format: one line per ref, tab-separated: `<name>\t<sha256>`.
/// Skips silently if refs dir is absent or empty.
pub fn write_cas_refs(rootfs_dir: &Path, overlay_dir: &Path) -> Result<()> {
    let refs = read_refs(overlay_dir)?;
    if refs.is_empty() {
        return Ok(());
    }

    let etc = rootfs_dir.join("etc");
    std::fs::create_dir_all(&etc).with_context(|| format!("creating {}", etc.display()))?;

    let mut content = String::new();
    for (name, hash) in &refs {
        content.push_str(name);
        content.push('\t');
        content.push_str(hash);
        content.push('\n');
    }

    let dest = etc.join("minibox-cas-refs");
    std::fs::write(&dest, &content).with_context(|| format!("writing {}", dest.display()))?;
    println!("  cas-refs {} ref(s) → {}", refs.len(), dest.display());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn sha256_file_produces_correct_digest() {
        let tmp = tempdir().unwrap();
        let file = tmp.path().join("data");
        // Known SHA-256 of "hello\n"
        std::fs::write(&file, b"hello\n").unwrap();
        let hash = sha256_file(&file).unwrap();
        assert_eq!(
            hash,
            "5891b5b522d5df086d0ff0b110fbd9d21bb4fc7163af34d08286a2e846f6be03"
        );
    }

    #[test]
    fn cas_add_creates_correct_file_layout() {
        let tmp = tempdir().unwrap();
        let overlay = tmp.path().join("overlay");
        let src = tmp.path().join("myfile.txt");
        std::fs::write(&src, b"test content").unwrap();

        let hash = cas_add(&overlay, &src, Some("myfile")).unwrap();

        // CAS object exists and content matches
        let cas_file = overlay.join("cas").join(&hash);
        assert!(cas_file.exists(), "cas/<hash> should exist");
        assert_eq!(std::fs::read(&cas_file).unwrap(), b"test content");

        // Ref file created
        let ref_file = overlay.join("refs").join("myfile");
        assert!(ref_file.exists(), "refs/myfile should exist");
        assert_eq!(std::fs::read_to_string(ref_file).unwrap(), hash);
    }

    #[test]
    fn cas_add_without_ref_skips_refs_dir() {
        let tmp = tempdir().unwrap();
        let overlay = tmp.path().join("overlay");
        let src = tmp.path().join("data");
        std::fs::write(&src, b"no ref").unwrap();

        cas_add(&overlay, &src, None).unwrap();

        assert!(
            !overlay.join("refs").exists(),
            "refs dir should not be created"
        );
        assert!(overlay.join("cas").exists(), "cas dir should exist");
    }

    #[test]
    fn cas_check_clean() {
        let tmp = tempdir().unwrap();
        let overlay = tmp.path().join("overlay");
        let src = tmp.path().join("file");
        std::fs::write(&src, b"clean content").unwrap();

        cas_add(&overlay, &src, Some("myref")).unwrap();
        // Check passes because CAS object matches the stored hash.
        assert!(cas_check(&overlay).is_ok());
    }

    #[test]
    fn cas_check_detects_drift() {
        let tmp = tempdir().unwrap();
        let overlay = tmp.path().join("overlay");
        let src = tmp.path().join("file");
        std::fs::write(&src, b"original").unwrap();

        let hash = cas_add(&overlay, &src, Some("drifted")).unwrap();

        // Corrupt the CAS object.
        let cas_file = overlay.join("cas").join(&hash);
        std::fs::write(&cas_file, b"tampered").unwrap();

        let result = cas_check(&overlay);
        assert!(result.is_err(), "cas_check should fail on drift");
    }

    #[test]
    fn cas_check_skips_missing_refs_dir() {
        let tmp = tempdir().unwrap();
        let overlay = tmp.path().join("overlay");
        std::fs::create_dir_all(&overlay).unwrap();
        // No refs dir — should return Ok silently.
        assert!(cas_check(&overlay).is_ok());
    }

    #[test]
    fn write_cas_refs_produces_tab_separated_file() {
        let tmp = tempdir().unwrap();
        let overlay = tmp.path().join("overlay");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).unwrap();

        // Add two refs
        let src = tmp.path().join("f");
        std::fs::write(&src, b"aaa").unwrap();
        let hash_a = cas_add(&overlay, &src, Some("alpha")).unwrap();
        std::fs::write(&src, b"bbb").unwrap();
        let hash_b = cas_add(&overlay, &src, Some("beta")).unwrap();

        write_cas_refs(&rootfs, &overlay).unwrap();

        let content = std::fs::read_to_string(rootfs.join("etc").join("minibox-cas-refs")).unwrap();
        // Sorted by name: alpha, beta
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], format!("alpha\t{}", hash_a));
        assert_eq!(lines[1], format!("beta\t{}", hash_b));
    }

    #[test]
    fn write_cas_refs_skips_when_no_refs() {
        let tmp = tempdir().unwrap();
        let overlay = tmp.path().join("overlay");
        let rootfs = tmp.path().join("rootfs");
        std::fs::create_dir_all(&rootfs).unwrap();
        std::fs::create_dir_all(&overlay).unwrap();

        write_cas_refs(&rootfs, &overlay).unwrap();

        // File should not exist
        assert!(!rootfs.join("etc").join("minibox-cas-refs").exists());
    }

    #[test]
    fn read_refs_returns_sorted_pairs() {
        let tmp = tempdir().unwrap();
        let overlay = tmp.path().join("overlay");
        let src = tmp.path().join("f");

        std::fs::write(&src, b"x").unwrap();
        cas_add(&overlay, &src, Some("zebra")).unwrap();
        std::fs::write(&src, b"y").unwrap();
        cas_add(&overlay, &src, Some("apple")).unwrap();

        let refs = read_refs(&overlay).unwrap();
        assert_eq!(refs[0].0, "apple");
        assert_eq!(refs[1].0, "zebra");
    }
}
