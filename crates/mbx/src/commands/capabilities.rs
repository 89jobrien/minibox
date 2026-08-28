//! `mbx capabilities` — query typed backend capabilities from the daemon.

use super::RequestError;
use anyhow::Context as _;
use minibox_core::client::DaemonClient;
use minibox_core::domain::CapabilityMatrix;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::fmt::Write as _;
use std::path::Path;

/// Render a stable human-readable capability table.
fn render_text(matrix: &CapabilityMatrix) -> Result<String, std::fmt::Error> {
    let mut output = String::from(
        "FEATURE                     native  gke     colima  smolvm  krun    vz      winbox\n",
    );
    let mut current_group = None;
    for row in &matrix.capabilities {
        if current_group != Some(row.group) {
            current_group = Some(row.group);
            writeln!(output, "\n{}", row.group.label())?;
        }
        write!(output, "{:<27}", row.capability.label())?;
        for backend in &matrix.backends {
            write!(output, " {:<7}", row.for_backend(*backend).label())?;
        }
        output.push('\n');
    }
    Ok(output)
}

/// Query and print the daemon capability matrix.
pub async fn execute(json: bool, socket_path: &Path) -> anyhow::Result<()> {
    let client = DaemonClient::with_socket(socket_path);
    let mut stream = client
        .call(DaemonRequest::GetCapabilities)
        .await
        .context("failed to query daemon capabilities")?;
    match stream
        .next()
        .await
        .context("capability response stream failed")?
    {
        Some(DaemonResponse::CapabilityMatrix { matrix }) => {
            if json {
                println!("{}", serde_json::to_string_pretty(&matrix)?);
            } else {
                print!("{}", render_text(&matrix)?);
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
    use crate::commands::test_helpers::setup;
    use minibox_core::domain::capability_matrix;

    #[test]
    fn human_output_is_capability_aware() {
        let output = render_text(&capability_matrix()).expect("render matrix");
        let pause = output
            .lines()
            .find(|line| line.starts_with("pause/resume"))
            .expect("pause row");
        let exec = output
            .lines()
            .find(|line| line.starts_with("exec (-it)"))
            .expect("exec row");
        let pid = output
            .lines()
            .find(|line| line.starts_with("PID namespace"))
            .expect("PID row");
        assert!(pause.contains("Yes") && pause.contains("No"));
        assert!(exec.contains("Yes") && exec.contains("Limited"));
        assert!(pid.contains("Lima VM") && pid.contains("VM"));
    }

    #[tokio::test]
    async fn execute_accepts_capability_matrix_response() {
        let (_tmp, socket_path) = setup(DaemonResponse::CapabilityMatrix {
            matrix: capability_matrix(),
        })
        .await;
        assert!(execute(false, &socket_path).await.is_ok());
    }
}
