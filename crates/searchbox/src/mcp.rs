//! JSON-RPC 2.0 stdio MCP server (MCP spec 2025-03-26).
//! Reads newline-delimited JSON from stdin, writes responses to stdout.

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::{self, BufRead, Write};
use tracing::debug;

use crate::domain::{SearchProvider, SearchQuery, ServiceManager};

// ---------------------------------------------------------------------------
// JSON-RPC wire types
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Request {
    id: Value,
    method: String,
    params: Option<Value>,
}

#[derive(Serialize)]
struct Response {
    jsonrpc: &'static str,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
}

#[derive(Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

impl Response {
    fn ok(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: Some(result),
            error: None,
        }
    }

    fn err(id: Value, code: i32, message: impl Into<String>) -> Self {
        Self {
            jsonrpc: "2.0",
            id,
            result: None,
            error: Some(RpcError {
                code,
                message: message.into(),
            }),
        }
    }
}

// ---------------------------------------------------------------------------
// MCP tool manifest
// ---------------------------------------------------------------------------

fn tools_list() -> Value {
    json!({
        "tools": [
            {
                "name": "search",
                "description": "Full-text search across indexed repos",
                "inputSchema": {
                    "type": "object",
                    "required": ["q"],
                    "properties": {
                        "q":              { "type": "string" },
                        "repos":          { "type": "array", "items": { "type": "string" } },
                        "lang":           { "type": "string" },
                        "case_sensitive": { "type": "boolean" },
                        "context_lines":  { "type": "integer", "minimum": 0, "maximum": 10 }
                    }
                }
            },
            {
                "name": "list_repos",
                "description": "List indexed repos and last-indexed timestamps",
                "inputSchema": { "type": "object", "properties": {} }
            },
            {
                "name": "reindex",
                "description": "Trigger reindex. Omit `repo` to reindex all.",
                "inputSchema": {
                    "type": "object",
                    "properties": {
                        "repo": { "type": "string" }
                    }
                }
            },
            {
                "name": "service_status",
                "description": "Check zoekt-webserver health on VPS",
                "inputSchema": { "type": "object", "properties": {} }
            }
        ]
    })
}

// ---------------------------------------------------------------------------
// Stdio loop
// ---------------------------------------------------------------------------

pub async fn run_stdio_loop(
    search: &dyn SearchProvider,
    service: &dyn ServiceManager,
) -> anyhow::Result<()> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        debug!(line = %line, "mcp: recv");

        let req: Request = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = Response::err(Value::Null, -32700, format!("parse error: {e}"));
                writeln!(out, "{}", serde_json::to_string(&resp)?)?;
                out.flush()?;
                continue;
            }
        };

        let resp = dispatch(&req, search, service).await;
        writeln!(out, "{}", serde_json::to_string(&resp)?)?;
        out.flush()?;
    }
    Ok(())
}

async fn dispatch(
    req: &Request,
    search: &dyn SearchProvider,
    service: &dyn ServiceManager,
) -> Response {
    let id = req.id.clone();
    let params = req.params.clone().unwrap_or(json!({}));

    match req.method.as_str() {
        "initialize" => Response::ok(
            id,
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "searchbox", "version": env!("CARGO_PKG_VERSION") }
            }),
        ),

        "tools/list" => Response::ok(id, tools_list()),

        "tools/call" => {
            let name = params["name"].as_str().unwrap_or("").to_string();
            let args = params["arguments"].clone();
            handle_tool_call(id, &name, &args, search, service).await
        }

        other => Response::err(id, -32601, format!("method not found: {other}")),
    }
}

async fn handle_tool_call(
    id: Value,
    name: &str,
    args: &Value,
    search: &dyn SearchProvider,
    service: &dyn ServiceManager,
) -> Response {
    match name {
        "search" => {
            let q = match args["q"].as_str() {
                Some(s) => s.to_string(),
                None => return Response::err(id, -32602, "missing `q`"),
            };
            let mut query = SearchQuery::new(q);
            if let Some(repos) = args["repos"].as_array() {
                query.repos = Some(
                    repos
                        .iter()
                        .filter_map(|v| v.as_str().map(String::from))
                        .collect(),
                );
            }
            if let Some(lang) = args["lang"].as_str() {
                query.lang = Some(lang.to_string());
            }
            if let Some(cs) = args["case_sensitive"].as_bool() {
                query.case_sensitive = cs;
            }
            if let Some(ctx) = args["context_lines"].as_u64() {
                query.context_lines = ctx.min(10) as u8;
            }
            match search.search(query).await {
                Ok(results) => {
                    let content: Vec<Value> = results
                        .iter()
                        .map(|r| {
                            json!({
                                "repo": r.repo, "file": r.file,
                                "line": r.line, "col": r.col,
                                "snippet": r.snippet, "score": r.score,
                                "commit": r.commit,
                            })
                        })
                        .collect();
                    Response::ok(
                        id,
                        json!({ "content": [{ "type": "text", "text": serde_json::to_string(&content).unwrap_or_default() }] }),
                    )
                }
                Err(e) => Response::err(id, -32000, e.to_string()),
            }
        }

        "list_repos" => match search.list_repos().await {
            Ok(repos) => {
                let content: Vec<Value> = repos
                    .iter()
                    .map(|r| {
                        json!({
                            "name": r.name,
                            "source_type": format!("{:?}", r.source_type),
                            "last_indexed": r.last_indexed.map(|t| t.to_rfc3339()),
                            "doc_count": r.doc_count,
                        })
                    })
                    .collect();
                Response::ok(
                    id,
                    json!({ "content": [{ "type": "text", "text": serde_json::to_string(&content).unwrap_or_default() }] }),
                )
            }
            Err(e) => Response::err(id, -32000, e.to_string()),
        },

        "reindex" => {
            let repo = args["repo"].as_str();
            match service.reindex(repo).await {
                Ok(()) => Response::ok(
                    id,
                    json!({ "content": [{ "type": "text", "text": "reindex triggered" }] }),
                ),
                Err(e) => Response::err(id, -32000, e.to_string()),
            }
        }

        "service_status" => match service.status().await {
            Ok(status) => Response::ok(
                id,
                json!({ "content": [{ "type": "text", "text": format!("{status:?}") }] }),
            ),
            Err(e) => Response::err(id, -32000, e.to_string()),
        },

        other => Response::err(id, -32601, format!("unknown tool: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{
        RepoInfo, SearchError, SearchResult, ServiceError, ServiceStatus, SourceType,
    };

    struct MockSearch {
        results: Vec<SearchResult>,
    }

    #[async_trait::async_trait]
    impl SearchProvider for MockSearch {
        async fn search(&self, _q: SearchQuery) -> Result<Vec<SearchResult>, SearchError> {
            Ok(self.results.clone())
        }

        async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError> {
            Ok(vec![RepoInfo {
                name: "testrepo".into(),
                source_type: SourceType::Git,
                last_indexed: None,
                doc_count: 42,
            }])
        }
    }

    struct MockService;

    #[async_trait::async_trait]
    impl ServiceManager for MockService {
        async fn start(&self) -> Result<(), ServiceError> {
            Ok(())
        }
        async fn stop(&self) -> Result<(), ServiceError> {
            Ok(())
        }
        async fn status(&self) -> Result<ServiceStatus, ServiceError> {
            Ok(ServiceStatus::Running)
        }
        async fn reindex(&self, _repo: Option<&str>) -> Result<(), ServiceError> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn dispatch_initialize_returns_protocol_version() {
        let req = Request {
            id: json!(1),
            method: "initialize".into(),
            params: None,
        };
        let search = MockSearch { results: vec![] };
        let service = MockService;
        let resp = dispatch(&req, &search, &service).await;
        let result = resp.result.expect("should have result");
        assert_eq!(result["protocolVersion"], "2025-03-26");
    }

    #[tokio::test]
    async fn dispatch_tools_list_returns_four_tools() {
        let req = Request {
            id: json!(2),
            method: "tools/list".into(),
            params: None,
        };
        let search = MockSearch { results: vec![] };
        let service = MockService;
        let resp = dispatch(&req, &search, &service).await;
        let tools = &resp.result.unwrap()["tools"];
        assert_eq!(tools.as_array().unwrap().len(), 4);
    }

    #[tokio::test]
    async fn dispatch_unknown_method_returns_error() {
        let req = Request {
            id: json!(3),
            method: "unknown/method".into(),
            params: None,
        };
        let search = MockSearch { results: vec![] };
        let service = MockService;
        let resp = dispatch(&req, &search, &service).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32601);
    }

    #[tokio::test]
    async fn search_tool_missing_q_returns_invalid_params() {
        let req = Request {
            id: json!(4),
            method: "tools/call".into(),
            params: Some(json!({ "name": "search", "arguments": {} })),
        };
        let search = MockSearch { results: vec![] };
        let service = MockService;
        let resp = dispatch(&req, &search, &service).await;
        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }
}
