use crate::domain::{
    CgroupNs, ContainerDetails, ContainerRuntime, ContainerSummary, CreateConfig, LogChunk,
    PullProgress, RuntimeError,
};
use anyhow::anyhow;
use async_trait::async_trait;
use minibox_client::DaemonClient;
use minibox_core::protocol::{DaemonRequest, DaemonResponse};
use tokio::sync::mpsc;

/// Pad a minibox short ID (16 hex chars) to a Docker-style 64-char ID.
fn to_docker_id(id: &str) -> String {
    format!("{:0<64}", id)
}

/// Truncate a Docker 64-char ID back to minibox short ID (16 chars).
fn to_minibox_id(id: &str) -> &str {
    &id[..16.min(id.len())]
}

pub struct MiniboxAdapter {
    socket_path: String,
}

impl MiniboxAdapter {
    pub fn new(socket_path: &str) -> Self {
        Self {
            socket_path: socket_path.to_string(),
        }
    }

    fn client(&self) -> DaemonClient {
        DaemonClient::with_socket(&self.socket_path)
    }
}

#[async_trait]
impl ContainerRuntime for MiniboxAdapter {
    async fn ping(&self) -> Result<(), RuntimeError> {
        // Attempt to connect; if List succeeds the daemon is up
        let mut stream = self.client().call(DaemonRequest::List).await?;
        stream.next().await?;
        Ok(())
    }

    async fn pull_image(
        &self,
        image: &str,
        tag: &str,
        tx: mpsc::Sender<PullProgress>,
    ) -> Result<(), RuntimeError> {
        let mut stream = self
            .client()
            .call(DaemonRequest::Pull {
                image: image.to_string(),
                tag: if tag == "latest" {
                    None
                } else {
                    Some(tag.to_string())
                },
            })
            .await?;

        // Drain responses; minibox pull doesn't stream progress, just send one event
        while let Some(resp) = stream.next().await? {
            match resp {
                DaemonResponse::Success { .. } => {
                    let _ = tx
                        .send(PullProgress {
                            status: "Pull complete".to_string(),
                            id: Some(format!("{}:{}", image, tag)),
                            progress: None,
                        })
                        .await;
                }
                DaemonResponse::Error { message } => {
                    return Err(RuntimeError::Minibox(anyhow!("{}", message)));
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn image_exists(&self, image: &str) -> Result<bool, RuntimeError> {
        // minibox doesn't have an image inspect endpoint; we check via pull-would-succeed
        // by listing containers and seeing if any use this image, or just return true
        // as a best-effort (pull handles missing images).
        // For now, treat all images as potentially present to avoid blocking create.
        let _ = image;
        Ok(true)
    }

    async fn create_container(&self, config: CreateConfig) -> Result<String, RuntimeError> {
        // Parse image:tag from config.image
        let (image, tag) = if let Some((img, t)) = config.image.split_once(':') {
            (img.to_string(), Some(t.to_string()))
        } else {
            (config.image.clone(), None)
        };

        // Convert binds ("host:container[:ro]") to BindMount structs
        let mounts: Vec<minibox_core::domain::BindMount> = config
            .binds
            .iter()
            .filter_map(|b| {
                let parts: Vec<&str> = b.splitn(3, ':').collect();
                if parts.len() < 2 {
                    return None;
                }
                Some(minibox_core::domain::BindMount {
                    host_path: parts[0].into(),
                    container_path: parts[1].into(),
                    read_only: parts.get(2).copied() == Some("ro"),
                })
            })
            .collect();

        let network = match config.network_mode.as_deref() {
            Some("host") => Some(minibox_core::domain::NetworkMode::Host),
            Some("bridge") => Some(minibox_core::domain::NetworkMode::Bridge),
            _ => None,
        };

        let cmd = if config.cmd.is_empty() {
            vec!["/bin/sh".to_string()]
        } else {
            config.cmd.clone()
        };

        let mut stream = self
            .client()
            .call(DaemonRequest::Run {
                image,
                tag,
                command: cmd,
                memory_limit_bytes: None,
                cpu_weight: None,
                ephemeral: false,
                network,
                env: config.env.clone(),
                mounts,
                privileged: config.privileged,
                name: None,
                tty: false,
            })
            .await?;

        while let Some(resp) = stream.next().await? {
            match resp {
                DaemonResponse::ContainerCreated { id } => {
                    return Ok(to_docker_id(&id));
                }
                DaemonResponse::Error { message } => {
                    return Err(RuntimeError::Minibox(anyhow!("{}", message)));
                }
                _ => {}
            }
        }

        Err(RuntimeError::Minibox(anyhow!(
            "no ContainerCreated response from miniboxd"
        )))
    }

    async fn start_container(&self, _id: &str) -> Result<(), RuntimeError> {
        // minibox starts containers immediately on Run; start is a no-op
        Ok(())
    }

    async fn inspect_container(&self, id: &str) -> Result<ContainerDetails, RuntimeError> {
        let minibox_id = to_minibox_id(id).to_string();
        let stream = self.client().call(DaemonRequest::List).await?;
        let responses = stream.try_collect().await?;

        for resp in responses {
            if let DaemonResponse::ContainerList { containers } = resp {
                for c in containers {
                    if c.id == minibox_id {
                        let status = match c.state.as_str() {
                            "running" => "running",
                            "stopped" => "exited",
                            "created" => "created",
                            _ => "dead",
                        };
                        return Ok(ContainerDetails {
                            id: to_docker_id(&c.id),
                            name: format!("/{}", c.id),
                            image: c.image.clone(),
                            status: status.to_string(),
                            exit_code: None,
                            created: c.created_at.clone(),
                            config: CreateConfig {
                                image: c.image,
                                name: None,
                                cmd: c.command.split_whitespace().map(str::to_string).collect(),
                                env: vec![],
                                binds: vec![],
                                ports: vec![],
                                privileged: false,
                                cgroup_ns: CgroupNs::Private,
                                network_mode: None,
                                labels: std::collections::HashMap::new(),
                            },
                        });
                    }
                }
            }
        }

        Err(RuntimeError::NotFound(format!("container {id} not found")))
    }

    async fn list_containers(&self, all: bool) -> Result<Vec<ContainerSummary>, RuntimeError> {
        let stream = self.client().call(DaemonRequest::List).await?;
        let responses = stream.try_collect().await?;

        let mut result = Vec::new();
        for resp in responses {
            if let DaemonResponse::ContainerList { containers } = resp {
                for c in containers {
                    let state = c.state.clone();
                    let status = match state.as_str() {
                        "running" => "Up".to_string(),
                        "stopped" => "Exited (0) 0 seconds ago".to_string(),
                        _ => state.clone(),
                    };

                    if !all && state != "running" {
                        continue;
                    }

                    result.push(ContainerSummary {
                        id: to_docker_id(&c.id),
                        names: vec![format!("/{}", c.id)],
                        image: c.image,
                        status,
                        state,
                    });
                }
            }
        }

        Ok(result)
    }

    async fn stream_logs(
        &self,
        id: &str,
        _follow: bool,
        tx: mpsc::Sender<LogChunk>,
    ) -> Result<(), RuntimeError> {
        // minibox streams logs via ephemeral run; here we re-run with the same ID
        // which isn't directly supported. Instead, we send a placeholder for now.
        let _ = id;
        let _ = tx
            .send(LogChunk {
                stream: 1,
                data: bytes::Bytes::from("(log streaming not supported for existing containers)\n"),
            })
            .await;
        Ok(())
    }

    async fn stop_container(&self, id: &str, _timeout_secs: u32) -> Result<(), RuntimeError> {
        let minibox_id = to_minibox_id(id).to_string();
        let mut stream = self
            .client()
            .call(DaemonRequest::Stop { id: minibox_id })
            .await?;

        while let Some(resp) = stream.next().await? {
            match resp {
                DaemonResponse::Success { .. } => return Ok(()),
                DaemonResponse::Error { message } => {
                    return Err(RuntimeError::Minibox(anyhow!("{}", message)));
                }
                _ => {}
            }
        }
        Ok(())
    }

    async fn wait_container(&self, id: &str) -> Result<i64, RuntimeError> {
        // Poll inspect until container is no longer running
        loop {
            match self.inspect_container(id).await {
                Ok(details) if details.status != "running" => {
                    return Ok(details.exit_code.unwrap_or(0));
                }
                Ok(_) => {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
                Err(RuntimeError::NotFound(_)) => return Ok(0),
                Err(e) => return Err(e),
            }
        }
    }

    async fn remove_container(&self, id: &str) -> Result<(), RuntimeError> {
        let minibox_id = to_minibox_id(id).to_string();
        let mut stream = self
            .client()
            .call(DaemonRequest::Remove { id: minibox_id })
            .await?;

        while let Some(resp) = stream.next().await? {
            match resp {
                DaemonResponse::Success { .. } => return Ok(()),
                DaemonResponse::Error { message } => {
                    return Err(RuntimeError::Minibox(anyhow!("{}", message)));
                }
                _ => {}
            }
        }
        Ok(())
    }
}
