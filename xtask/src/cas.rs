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
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("/tmp"))
        .join(".minibox")
        .join("vm")
        .join("overlay")
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
}
