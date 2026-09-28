//! Pure parsing and path classification for image-declared volumes.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

#[derive(Deserialize)]
struct ImageConfigEnvelope {
    #[serde(default)]
    config: RuntimeConfig,
}

#[derive(Default, Deserialize)]
struct RuntimeConfig {
    #[serde(rename = "Volumes", alias = "volumes", default)]
    volumes: Option<BTreeMap<String, serde_json::Value>>,
}

/// Parse and validate absolute volume paths from an OCI/Docker image config blob.
///
/// # Errors
///
/// Returns an error for malformed JSON or unsafe volume paths.
pub fn parse_volume_paths(config: &[u8]) -> Result<Vec<PathBuf>> {
    let envelope: ImageConfigEnvelope =
        serde_json::from_slice(config).context("parse image config volume declarations")?;
    envelope
        .config
        .volumes
        .unwrap_or_default()
        .into_keys()
        .map(|raw| validate_volume_path(&raw))
        .collect()
}

pub(crate) fn validate_volume_path(raw: &str) -> Result<PathBuf> {
    let path = PathBuf::from(raw);
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::CurDir | Component::Prefix(_)
            )
        })
    {
        bail!("invalid image volume path {raw:?}: expected a normalized absolute path");
    }
    Ok(path)
}

/// Return whether a rootfs-relative path is equal to or below a declared volume.
#[must_use]
pub fn path_is_volume_masked(path: &Path, volumes: &[PathBuf]) -> bool {
    let relative = path.strip_prefix("/").unwrap_or(path);
    volumes.iter().any(|volume| {
        volume
            .strip_prefix("/")
            .is_ok_and(|volume_relative| relative.starts_with(volume_relative))
    })
}

#[cfg(test)]
mod tests {
    use super::{parse_volume_paths, path_is_volume_masked};
    use std::path::{Path, PathBuf};

    #[test]
    fn parses_docker_image_config_volumes() {
        let config = b"{\"config\":{\"Volumes\":{\"/var/lib/docker\":{},\"/data\":null}}}";
        assert_eq!(
            parse_volume_paths(config).expect("parse image config"),
            vec![PathBuf::from("/data"), PathBuf::from("/var/lib/docker")]
        );
    }

    #[test]
    fn missing_or_null_volumes_are_empty() {
        assert!(
            parse_volume_paths(b"{\"config\":{}}")
                .expect("parse")
                .is_empty()
        );
        assert!(
            parse_volume_paths(b"{\"config\":{\"Volumes\":null}}")
                .expect("parse")
                .is_empty()
        );
    }

    #[test]
    fn rejects_relative_or_parent_volume_paths() {
        for config in [
            b"{\"config\":{\"Volumes\":{\"data\":{}}}}".as_slice(),
            b"{\"config\":{\"Volumes\":{\"/var/../secret\":{}}}}".as_slice(),
        ] {
            assert!(parse_volume_paths(config).is_err());
        }
    }

    #[test]
    fn detects_volume_root_and_descendants_only() {
        let volumes = vec![PathBuf::from("/var/lib/docker")];
        assert!(path_is_volume_masked(Path::new("var/lib/docker"), &volumes));
        assert!(path_is_volume_masked(
            Path::new("var/lib/docker/overlay2/layer"),
            &volumes
        ));
        assert!(!path_is_volume_masked(
            Path::new("var/lib/dockerd"),
            &volumes
        ));
        assert!(!path_is_volume_masked(Path::new("etc/config"), &volumes));
    }
}
