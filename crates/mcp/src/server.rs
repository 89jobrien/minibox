//! MCP server definition and tool routing.

use crate::client::MiniboxDaemonClient;
use crate::policy::AgentPolicy;
use crate::tools::{containers, doctor, images};
use crate::types::{
    ContainerIdInput, DoctorInput, DoctorOutput, EmptyInput, ImagesOutput, LogsInput, LogsOutput,
    ManifestOutput, PsOutput, PullImageInput, PullImageOutput, RunContainerInput,
    RunContainerOutput, SimpleOutput,
};
use rmcp::handler::server::{router::tool::ToolRouter, wrapper::Parameters};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, Json, ServerHandler, tool, tool_handler, tool_router};

/// MCP server exposing minibox daemon operations.
#[derive(Clone, Debug)]
pub struct MiniboxMcpServer {
    client: MiniboxDaemonClient,
    policy: AgentPolicy,
    tool_router: ToolRouter<Self>,
}

#[tool_router(router = tool_router)]
impl MiniboxMcpServer {
    /// Create a server from explicit dependencies.
    #[must_use]
    pub fn new(client: MiniboxDaemonClient, policy: AgentPolicy) -> Self {
        Self {
            client,
            policy,
            tool_router: Self::tool_router(),
        }
    }

    /// Create a server from environment configuration.
    #[must_use]
    pub fn from_env() -> Self {
        Self::new(MiniboxDaemonClient::from_env(), AgentPolicy::from_env())
    }

    /// Check daemon connectivity.
    #[tool(
        name = "minibox_doctor",
        description = "Check minibox daemon socket connectivity"
    )]
    pub async fn minibox_doctor(
        &self,
        input: Parameters<DoctorInput>,
    ) -> Result<Json<DoctorOutput>, McpError> {
        Ok(Json(doctor::doctor(&self.client, input.0).await))
    }

    // TODO(review): no tracing::info!/warn! anywhere in this crate — tool invocations,
    // policy denials, and daemon errors are only ever visible to the calling agent as a
    // string, with no server-side breadcrumb to debug from. Add structured tracing on
    // each tool entry/error path (key = value fields per repo convention).
    /// List known containers.
    #[tool(name = "minibox_ps", description = "List containers known to miniboxd")]
    pub async fn minibox_ps(
        &self,
        _input: Parameters<EmptyInput>,
    ) -> Result<Json<PsOutput>, String> {
        containers::ps(&self.client, &self.policy)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    /// List cached images.
    #[tool(name = "minibox_images", description = "List cached minibox images")]
    pub async fn minibox_images(
        &self,
        _input: Parameters<EmptyInput>,
    ) -> Result<Json<ImagesOutput>, String> {
        images::list_images(&self.client, &self.policy)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    /// Fetch stored container logs.
    #[tool(
        name = "minibox_logs",
        description = "Fetch stored stdout/stderr logs for a container"
    )]
    pub async fn minibox_logs(
        &self,
        input: Parameters<LogsInput>,
    ) -> Result<Json<LogsOutput>, String> {
        containers::logs(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    /// Get a container execution manifest.
    #[tool(
        name = "minibox_manifest",
        description = "Get a container execution manifest"
    )]
    pub async fn minibox_manifest(
        &self,
        input: Parameters<ContainerIdInput>,
    ) -> Result<Json<ManifestOutput>, String> {
        containers::manifest(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    /// Pull an image through miniboxd.
    #[tool(
        name = "minibox_pull",
        description = "Pull an OCI image through miniboxd"
    )]
    pub async fn minibox_pull(
        &self,
        input: Parameters<PullImageInput>,
    ) -> Result<Json<PullImageOutput>, String> {
        images::pull_image(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    /// Run a container with agent-safe defaults.
    #[tool(
        name = "minibox_run",
        description = "Run a container with bounded output collection"
    )]
    pub async fn minibox_run(
        &self,
        input: Parameters<RunContainerInput>,
    ) -> Result<Json<RunContainerOutput>, String> {
        containers::run(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    /// Stop a running container.
    #[tool(
        name = "minibox_stop",
        description = "Stop a running container by ID or name"
    )]
    pub async fn minibox_stop(
        &self,
        input: Parameters<ContainerIdInput>,
    ) -> Result<Json<SimpleOutput>, String> {
        containers::stop(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(Into::into)
    }

    /// Remove a stopped container.
    #[tool(
        name = "minibox_rm",
        description = "Remove a stopped container by ID or name"
    )]
    pub async fn minibox_rm(
        &self,
        input: Parameters<ContainerIdInput>,
    ) -> Result<Json<SimpleOutput>, String> {
        containers::rm(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(Into::into)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MiniboxMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "Control a local minibox daemon through safe, typed MCP tools.".into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_lists_expected_tools() {
        let server = MiniboxMcpServer::new(
            MiniboxDaemonClient::new("/tmp/minibox.sock".into()),
            AgentPolicy::safe_default(),
        );
        let tools = server.tool_router.list_all();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();

        assert!(names.contains(&"minibox_doctor"));
        assert!(names.contains(&"minibox_ps"));
        assert!(names.contains(&"minibox_run"));
        assert!(names.contains(&"minibox_rm"));
    }
}
