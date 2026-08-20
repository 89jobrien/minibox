//! MCP server definition and tool routing.

use crate::client::MiniboxDaemonClient;
use crate::error::McpServerError;
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

/// Log a failed tool call server-side, then convert it to a structured MCP error.
fn tool_error(tool: &'static str, error: McpServerError) -> McpError {
    tracing::warn!(
        tool = tool,
        code = error.diagnostic_code(),
        error = %error,
        "mcp: tool call failed"
    );
    error.into()
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

    /// Render a startup banner describing the daemon socket and available tools.
    #[must_use]
    pub fn banner(&self) -> String {
        const TITLE: &str = r"
       _      _ _
 _ __ (_)_ _ (_) |__  _____ __
| '  \| | ' \| | '_ \/ _ \ \ /
|_|_|_|_|_||_|_|_.__/\___/_\_\    MCP control server
";

        let mut tools: Vec<String> = self
            .tool_router
            .list_all()
            .iter()
            .map(|tool| tool.name.to_string())
            .collect();
        tools.sort_unstable();

        use std::fmt::Write as _;

        let mut out = String::from(TITLE);
        let _ = write!(
            out,
            "transport   : stdio (MCP over stdin/stdout)\n\
             daemon      : {}\n\
             tools ({}) :\n",
            self.client.socket_path.display(),
            tools.len(),
        );
        for name in tools {
            let _ = writeln!(out, "  - {name}");
        }
        out
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
        tracing::debug!(tool = "minibox_doctor", "mcp: tool invoked");
        Ok(Json(
            doctor::doctor(&self.client, &self.policy, input.0).await,
        ))
    }

    /// List known containers.
    #[tool(name = "minibox_ps", description = "List containers known to miniboxd")]
    pub async fn minibox_ps(
        &self,
        _input: Parameters<EmptyInput>,
    ) -> Result<Json<PsOutput>, McpError> {
        tracing::debug!(tool = "minibox_ps", "mcp: tool invoked");
        containers::ps(&self.client, &self.policy)
            .await
            .map(Json)
            .map_err(|error| tool_error("minibox_ps", error))
    }

    /// List cached images.
    #[tool(name = "minibox_images", description = "List cached minibox images")]
    pub async fn minibox_images(
        &self,
        _input: Parameters<EmptyInput>,
    ) -> Result<Json<ImagesOutput>, McpError> {
        tracing::debug!(tool = "minibox_images", "mcp: tool invoked");
        images::list_images(&self.client, &self.policy)
            .await
            .map(Json)
            .map_err(|error| tool_error("minibox_images", error))
    }

    /// Fetch stored container logs.
    #[tool(
        name = "minibox_logs",
        description = "Fetch stored stdout/stderr logs for a container"
    )]
    pub async fn minibox_logs(
        &self,
        input: Parameters<LogsInput>,
    ) -> Result<Json<LogsOutput>, McpError> {
        tracing::debug!(tool = "minibox_logs", id = %input.0.id, "mcp: tool invoked");
        containers::logs(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(|error| tool_error("minibox_logs", error))
    }

    /// Get a container execution manifest.
    #[tool(
        name = "minibox_manifest",
        description = "Get a container execution manifest"
    )]
    pub async fn minibox_manifest(
        &self,
        input: Parameters<ContainerIdInput>,
    ) -> Result<Json<ManifestOutput>, McpError> {
        tracing::debug!(tool = "minibox_manifest", id = %input.0.id, "mcp: tool invoked");
        containers::manifest(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(|error| tool_error("minibox_manifest", error))
    }

    /// Pull an image through miniboxd.
    #[tool(
        name = "minibox_pull",
        description = "Pull an OCI image through miniboxd"
    )]
    pub async fn minibox_pull(
        &self,
        input: Parameters<PullImageInput>,
    ) -> Result<Json<PullImageOutput>, McpError> {
        tracing::debug!(tool = "minibox_pull", image = %input.0.image, "mcp: tool invoked");
        images::pull_image(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(|error| tool_error("minibox_pull", error))
    }

    /// Run a container with agent-safe defaults.
    #[tool(
        name = "minibox_run",
        description = "Run a container with bounded output collection"
    )]
    pub async fn minibox_run(
        &self,
        input: Parameters<RunContainerInput>,
    ) -> Result<Json<RunContainerOutput>, McpError> {
        tracing::debug!(tool = "minibox_run", image = %input.0.image, "mcp: tool invoked");
        containers::run(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(|error| tool_error("minibox_run", error))
    }

    /// Stop a running container.
    #[tool(
        name = "minibox_stop",
        description = "Stop a running container by ID or name"
    )]
    pub async fn minibox_stop(
        &self,
        input: Parameters<ContainerIdInput>,
    ) -> Result<Json<SimpleOutput>, McpError> {
        tracing::debug!(tool = "minibox_stop", id = %input.0.id, "mcp: tool invoked");
        containers::stop(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(|error| tool_error("minibox_stop", error))
    }

    /// Remove a stopped container.
    #[tool(
        name = "minibox_rm",
        description = "Remove a stopped container by ID or name"
    )]
    pub async fn minibox_rm(
        &self,
        input: Parameters<ContainerIdInput>,
    ) -> Result<Json<SimpleOutput>, McpError> {
        tracing::debug!(tool = "minibox_rm", id = %input.0.id, "mcp: tool invoked");
        containers::rm(&self.client, &self.policy, input.0)
            .await
            .map(Json)
            .map_err(|error| tool_error("minibox_rm", error))
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for MiniboxMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions("Control a local minibox daemon through safe, typed MCP tools.")
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
