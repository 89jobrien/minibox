//! `minibox load` — load an image from a local OCI tar archive.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Execute the `load` subcommand.
///
/// Sends a `LoadImage` request to the daemon with the given tar path, name,
/// and tag.  Prints `"Loaded image: <image>"` on success.
pub async fn execute(
    path: String,
    name: String,
    tag: String,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    let request = DaemonRequest::LoadImage {
        path: path.clone(),
        name: name.clone(),
        tag: tag.clone(),
    };

    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(request)
        .await
        .context("failed to call daemon")?;

    if let Some(response) = stream.next().await.context("stream error")? {
        match response {
            DaemonResponse::ImageLoaded { image } => {
                println!("Loaded image: {image}");
                Ok(())
            }
            DaemonResponse::Error { message } => {
                eprintln!("error: {message}");
                std::process::exit(1);
            }
            other => {
                eprintln!("unexpected response: {other:?}");
                std::process::exit(1);
            }
        }
    } else {
        eprintln!("no response from daemon");
        std::process::exit(1);
    }
}

/// Derive an image name from a file path by stripping the directory and
/// known archive extensions (`.tar`, `.tar.gz`, `.tgz`).
pub fn name_from_path(path: &str) -> String {
    let stem = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        // Fallback to full path string — safe on Linux/macOS where paths are valid UTF-8.
        .unwrap_or(path);

    // Strip known archive extensions.
    for ext in &[".tar.gz", ".tgz", ".tar"] {
        if let Some(stripped) = stem.strip_suffix(ext) {
            return stripped.to_string();
        }
    }
    stem.to_string()
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::setup;
    use super::*;

    #[test]
    fn name_from_path_strips_tar_gz() {
        assert_eq!(name_from_path("/tmp/myimage.tar.gz"), "myimage");
    }

    #[test]
    fn name_from_path_strips_tgz() {
        assert_eq!(name_from_path("myimage.tgz"), "myimage");
    }

    #[test]
    fn name_from_path_strips_tar() {
        assert_eq!(name_from_path("myimage.tar"), "myimage");
    }

    #[test]
    fn name_from_path_no_extension() {
        assert_eq!(name_from_path("myimage"), "myimage");
    }

    // ── Property-based tests ──────────────────────────────────────────────────

    use proptest::prelude::*;

    fn arb_stem() -> impl Strategy<Value = String> {
        "[a-z][a-z0-9_-]{1,15}"
    }

    proptest! {
        #![proptest_config(ProptestConfig { failure_persistence: None, ..ProptestConfig::default() })]

        /// Archive extensions are always stripped.
        #[test]
        fn prop_name_from_path_strips_archive_extension(
            stem in arb_stem(),
            ext in prop_oneof![Just(".tar.gz"), Just(".tgz"), Just(".tar")],
        ) {
            let filename = format!("{stem}{ext}");
            let result = name_from_path(&filename);
            prop_assert_eq!(&result, &stem);
        }

        /// Result is never empty for archive filenames.
        #[test]
        fn prop_name_from_path_nonempty(
            stem in arb_stem(),
            ext in prop_oneof![Just(".tar.gz"), Just(".tgz"), Just(".tar")],
        ) {
            let result = name_from_path(&format!("{stem}{ext}"));
            prop_assert!(!result.is_empty());
        }

        /// Directory prefix does not affect the result.
        #[test]
        fn prop_name_from_path_ignores_directory(
            dir in "[a-z]{1,10}".prop_map(|s| format!("/{s}")),
            stem in arb_stem(),
            ext in prop_oneof![Just(".tar.gz"), Just(".tgz"), Just(".tar")],
        ) {
            let filename = format!("{stem}{ext}");
            let with_dir = format!("{dir}/{filename}");
            prop_assert_eq!(name_from_path(&with_dir), name_from_path(&filename));
        }

        /// A stem with no known extension is returned as-is (no directory).
        #[test]
        fn prop_name_from_path_no_known_ext_unchanged(stem in arb_stem()) {
            // Stems from arb_stem() never end with .tar/.tgz/.tar.gz
            prop_assert_eq!(name_from_path(&stem), stem);
        }
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn execute_succeeds_on_image_loaded_response() {
        let (_tmp, socket_path) = setup(DaemonResponse::ImageLoaded {
            image: "myimage:latest".to_string(),
        })
        .await;
        let result = execute(
            "/tmp/myimage.tar".to_string(),
            "myimage".to_string(),
            "latest".to_string(),
            &socket_path,
        )
        .await;
        assert!(result.is_ok(), "execute should succeed: {result:?}");
    }
}
