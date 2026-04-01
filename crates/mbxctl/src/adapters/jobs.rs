use crate::client::DaemonClient;
use crate::error::ControllerError;
use crate::models::CreateJobRequest;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use std::sync::Arc;
use std::time::Duration;

pub struct JobAdapter {
    client: Arc<DaemonClient>,
}

impl JobAdapter {
    pub fn new(client: Arc<DaemonClient>) -> Self {
        Self { client }
    }

    /// Send a Run request and wait for `ContainerCreated`.
    ///
    /// Returns the container ID and the live [`ResponseStream`] so the caller
    /// can drain `ContainerOutput` / `ContainerStopped` messages.
    pub async fn create_and_run(
        &self,
        req: CreateJobRequest,
    ) -> Result<(String, crate::client::ResponseStream), ControllerError> {
        let daemon_req = DaemonRequest::Run {
            image: req.image,
            tag: req.tag.or(Some("latest".to_string())),
            command: req.command,
            memory_limit_bytes: req.memory_limit_bytes,
            cpu_weight: req.cpu_weight,
            ephemeral: true,
            network: None,
            mounts: vec![],
            privileged: false,
            env: req.env,
            name: None,
        };

        let mut stream = self
            .client
            .call(daemon_req)
            .await
            .map_err(|e| ControllerError::DaemonUnavailable(e.to_string()))?;

        // Wait for ContainerCreated to get the container ID, then hand the
        // stream back to the caller so it can drain output messages.
        loop {
            match stream
                .next()
                .await
                .map_err(|e| ControllerError::Internal(e.to_string()))?
            {
                Some(DaemonResponse::ContainerCreated { id }) => {
                    return Ok((id, stream));
                }
                Some(DaemonResponse::Error { message }) => {
                    return Err(ControllerError::ContainerFailed { message });
                }
                Some(_) => continue,
                None => {
                    return Err(ControllerError::Internal(
                        "stream closed before ContainerCreated".to_string(),
                    ));
                }
            }
        }
    }

    /// Run [`create_and_run`] with a timeout on the startup phase.
    ///
    /// Defaults to 3600 s (1 hour) when `timeout_seconds` is `None`.
    pub async fn create_and_run_with_timeout(
        &self,
        req: CreateJobRequest,
    ) -> Result<(String, crate::client::ResponseStream), ControllerError> {
        let timeout_secs = req.timeout_seconds.unwrap_or(3600);

        tokio::time::timeout(Duration::from_secs(timeout_secs), self.create_and_run(req))
            .await
            .map_err(|_| ControllerError::Timeout("job exceeded timeout".to_string()))?
    }

    pub async fn stop_container(&self, container_id: &str) -> Result<(), ControllerError> {
        let daemon_req = DaemonRequest::Stop {
            id: container_id.to_string(),
        };

        let mut stream = self
            .client
            .call(daemon_req)
            .await
            .map_err(|e| ControllerError::DaemonUnavailable(e.to_string()))?;

        match stream
            .next()
            .await
            .map_err(|e| ControllerError::Internal(e.to_string()))?
        {
            Some(DaemonResponse::Success { .. }) => Ok(()),
            Some(DaemonResponse::Error { message }) => {
                Err(ControllerError::ContainerFailed { message })
            }
            _ => Err(ControllerError::Internal("unexpected response".to_string())),
        }
    }
}
