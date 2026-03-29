use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[allow(dead_code)]
pub const ALPINE_VERSION: &str = "3.21.3";
#[allow(dead_code)]
pub const ALPINE_ARCH: &str = "aarch64";
#[allow(dead_code)]
pub const ALPINE_CDN: &str = "https://dl-cdn.alpinelinux.org/alpine";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[allow(dead_code)]
pub struct VmImageManifest {
    pub alpine_version: String,
    pub agent_rustc_version: String,
    pub agent_commit: String,
    pub built_at: u64, // Unix timestamp seconds
}

#[allow(dead_code)]
impl VmImageManifest {
    pub fn load(path: &Path) -> Result<Option<Self>> {
        if !path.exists() {
            return Ok(None);
        }
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("reading manifest {}", path.display()))?;
        let m: Self = serde_json::from_str(&text)
            .with_context(|| format!("parsing manifest {}", path.display()))?;
        Ok(Some(m))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating dir {}", parent.display()))?;
        }
        let json = serde_json::to_string_pretty(self).context("serializing manifest")?;
        std::fs::write(path, json)
            .with_context(|| format!("writing manifest {}", path.display()))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_roundtrip() {
        let m = VmImageManifest {
            alpine_version: "3.21.0".into(),
            agent_rustc_version: "1.87.0".into(),
            agent_commit: "abc1234".into(),
            built_at: 1711670400,
        };
        let json = serde_json::to_string(&m).unwrap();
        let back: VmImageManifest = serde_json::from_str(&json).unwrap();
        assert_eq!(back.alpine_version, "3.21.0");
        assert_eq!(back.agent_commit, "abc1234");
    }

    #[test]
    fn manifest_load_returns_none_for_missing_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        let result = VmImageManifest::load(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn manifest_save_and_load_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("manifest.json");
        let m = VmImageManifest {
            alpine_version: "3.21.3".into(),
            agent_rustc_version: "rustc 1.87.0".into(),
            agent_commit: "deadbeef".into(),
            built_at: 9999,
        };
        m.save(&path).unwrap();
        let loaded = VmImageManifest::load(&path).unwrap().unwrap();
        assert_eq!(loaded, m);
    }
}
