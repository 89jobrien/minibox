//! `mbx commit` — capture a container writable layer as an image.

use anyhow::Context;
use minibox_core::client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};

use super::RequestError;

/// Options accepted by the commit command.
pub struct CommitOpts {
    pub container_id: String,
    pub target_image: String,
    pub author: Option<String>,
    pub message: Option<String>,
    pub env_overrides: Vec<String>,
    pub cmd_override: Option<Vec<String>>,
    pub include_volumes: bool,
}

impl CommitOpts {
    fn into_request(self) -> DaemonRequest {
        DaemonRequest::Commit {
            container_id: self.container_id,
            target_image: self.target_image,
            author: self.author,
            message: self.message,
            env_overrides: self.env_overrides,
            cmd_override: self.cmd_override,
            include_volumes: self.include_volumes,
        }
    }
}

fn partition_message(message: &str) -> (Vec<&str>, Vec<&str>) {
    message
        .lines()
        .partition(|line| line.starts_with("warning: "))
}

/// Commit a container and print exclusion warnings separately from completion.
pub async fn execute(opts: CommitOpts, socket_path: &std::path::Path) -> anyhow::Result<()> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(opts.into_request())
        .await
        .context("failed to call daemon")?;

    match stream.next().await.context("stream error")? {
        Some(DaemonResponse::Success { message }) => {
            let (warnings, output) = partition_message(&message);
            for warning in warnings {
                eprintln!("{warning}");
            }
            for line in output {
                println!("{line}");
            }
            Ok(())
        }
        Some(DaemonResponse::Error { message }) => {
            Err(RequestError::DaemonError { message }.into())
        }
        Some(other) => Err(RequestError::UnexpectedResponse {
            response: format!("{other:?}"),
        }
        .into()),
        None => Err(RequestError::NoResponse.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn partitions_volume_warning_for_stderr() {
        let (warnings, output) = partition_message(
            "warning: image VOLUME /data contains data\ncommitted example:v1 digest:sha256:abc",
        );
        assert_eq!(warnings, vec!["warning: image VOLUME /data contains data"]);
        assert_eq!(output, vec!["committed example:v1 digest:sha256:abc"]);
    }

    #[test]
    fn request_carries_include_volumes_option() {
        let request = CommitOpts {
            container_id: "abc123".to_string(),
            target_image: "example:v1".to_string(),
            author: None,
            message: None,
            env_overrides: vec![],
            cmd_override: None,
            include_volumes: true,
        }
        .into_request();

        assert!(matches!(
            request,
            DaemonRequest::Commit {
                include_volumes: true,
                ..
            }
        ));
    }
}
