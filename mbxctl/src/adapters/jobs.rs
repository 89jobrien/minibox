use crate::client::DaemonClient;
use crate::error::ControllerError;
use crate::models::CreateJobRequest;
use linuxbox::protocol::{DaemonRequest, DaemonResponse};
use std::sync::Arc;

pub struct JobAdapter {
    client: Arc<DaemonClient>,
}

impl JobAdapter {
    pub fn new(client: Arc<DaemonClient>) -> Self {
        Self { client }
    }

    pub async fn create_and_run(
        &self,
        req: CreateJobRequest,
    ) -> Result<(String, String), ControllerError> {
        let daemon_req = DaemonRequest::Run {
            image: req.image,
            tag: req.tag.or(Some("latest".to_string())),
            command: req.command,
            memory_limit_bytes: req.memory_limit_bytes,
            cpu_weight: req.cpu_weight,
            ephemeral: true,
            network: None,
        };

        let mut stream = self
            .client
            .call(daemon_req)
            .await
            .map_err(|e| ControllerError::DaemonUnavailable(e.to_string()))?;

        // Wait for ContainerCreated to get the container ID
        loop {
            match stream
                .next()
                .await
                .map_err(|e| ControllerError::Internal(e.to_string()))?
            {
                Some(DaemonResponse::ContainerCreated { id }) => {
                    return Ok((id.clone(), id));
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
