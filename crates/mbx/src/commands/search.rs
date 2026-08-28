//! `mbx search` — discover cached image repositories and tags.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::image::search::ImageSearchResult;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

/// Format search results as a compact deterministic table.
#[must_use]
pub fn format_results(results: &[ImageSearchResult]) -> String {
    if results.is_empty() {
        return "(no images)".to_string();
    }

    let mut lines = vec![format!("{:<40}  {:<10}  {}", "NAME", "SOURCE", "TAGS")];
    lines.push("-".repeat(72));
    lines.extend(results.iter().map(|result| {
        format!(
            "{:<40}  {:<10}  {}",
            result.name,
            format!("{:?}", result.source).to_ascii_lowercase(),
            result.tags.join(",")
        )
    }));
    lines.join("\n")
}

/// Execute a local or remote image search through the daemon.
pub async fn execute(
    query: String,
    remote: bool,
    limit: usize,
    socket_path: &std::path::Path,
) -> anyhow::Result<()> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::SearchImages {
            query,
            remote,
            limit,
        })
        .await
        .context("failed to call daemon")?;

    match stream.next().await.context("stream error")? {
        Some(DaemonResponse::SearchResults { results }) => {
            println!("{}", format_results(&results));
            Ok(())
        }
        Some(DaemonResponse::Error { message }) => {
            Err(super::RequestError::DaemonError { message }.into())
        }
        Some(other) => Err(super::RequestError::UnexpectedResponse {
            response: format!("{other:?}"),
        }
        .into()),
        None => Err(super::RequestError::NoResponse.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_helpers::setup;
    use super::*;
    use minibox_core::image::search::SearchSource;

    #[test]
    fn formats_repository_tags_and_source() {
        let output = format_results(&[ImageSearchResult {
            name: "library/alpine".into(),
            tags: vec!["3.19".into(), "latest".into()],
            source: SearchSource::Local,
        }]);
        assert!(output.contains("library/alpine"));
        assert!(output.contains("3.19,latest"));
        assert!(output.contains("local"));
    }

    #[tokio::test]
    async fn execute_accepts_search_results() {
        let (_tmp, socket_path) = setup(DaemonResponse::SearchResults { results: vec![] }).await;
        let result = execute("alpine".into(), false, 25, &socket_path).await;
        assert!(result.is_ok(), "search should succeed: {result:?}");
    }
}
