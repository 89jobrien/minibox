# searchbox — Design Spec

**Date:** 2026-04-22
**Status:** Draft
**Crate:** `crates/searchbox`

---

## Overview

`searchbox` is a minibox-managed service adapter that provisions a [Zoekt][zoekt] full-text
search instance on the VPS and exposes a `SearchProvider` port for use by a local MCP stdio
server. Claude Code sessions query it for code search, git history traversal, and doc search
(Obsidian vault + markdown) without leaving the session.

The crate follows minibox's hexagonal architecture: domain traits (ports) in `domain.rs`,
infrastructure adapters in `adapters/`, composition in `bin/searchboxd.rs`.

---

## Ports (Domain Traits)

### `SearchProvider`

The primary query port. All query consumers depend on this trait only.

```rust
#[async_trait]
pub trait SearchProvider: Send + Sync {
    async fn search(&self, query: SearchQuery) -> Result<Vec<SearchResult>, SearchError>;
    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError>;
}
```

### `IndexSource`

Describes a corpus that can be indexed. Each variant is a separate adapter.

```rust
#[async_trait]
pub trait IndexSource: Send + Sync {
    fn name(&self) -> &str;
    fn source_type(&self) -> SourceType;  // Git | Filesystem | Local
    async fn sync(&self, dest: &Path) -> Result<SyncStats, IndexError>;
}
```

### `ServiceManager`

Lifecycle management for the Zoekt process on the VPS. Implemented by
`ZoektServiceAdapter`, which shells out via SSH through `tailbox`.

```rust
#[async_trait]
pub trait ServiceManager: Send + Sync {
    async fn start(&self) -> Result<(), ServiceError>;
    async fn stop(&self) -> Result<(), ServiceError>;
    async fn status(&self) -> Result<ServiceStatus, ServiceError>;
    async fn reindex(&self, repo: Option<&str>) -> Result<(), ServiceError>;
}
```

---

## Domain Types

```rust
pub struct SearchQuery {
    pub text: String,
    pub repos: Option<Vec<String>>,   // None = all repos
    pub lang: Option<String>,
    pub case_sensitive: bool,
    pub context_lines: u8,            // default 2
}

pub struct SearchResult {
    pub repo: String,
    pub file: String,
    pub line: u32,
    pub snippet: String,
    pub score: f32,
}

pub struct RepoInfo {
    pub name: String,
    pub source_type: SourceType,
    pub last_indexed: Option<DateTime<Utc>>,
}

pub enum SourceType { Git, Filesystem, Local }
pub enum ServiceStatus { Running, Stopped, Indexing }

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("zoekt unavailable: {0}")]
    Unavailable(String),
    #[error("query failed: {0}")]
    QueryFailed(String),
}

#[derive(Debug, Error)]
pub enum IndexError {
    #[error("sync failed for {repo}: {reason}")]
    SyncFailed { repo: String, reason: String },
}

#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("ssh error: {0}")]
    Ssh(String),
    #[error("zoekt process error: {0}")]
    Process(String),
}
```

---

## Adapters

### `ZoektAdapter` — `SearchProvider`

HTTP client to `zoekt-webserver` running on the VPS (default port 6070). Translates
`SearchQuery` into Zoekt's JSON search API. Does not know how repos got indexed.

### `MergedAdapter` — `SearchProvider`

Fan-out across a `Vec<Box<dyn SearchProvider>>`. Runs queries concurrently via
`tokio::join_all`, merges results by score, deduplicates by `(repo, file, line)`.
Used to combine VPS Zoekt with an optional local sidecar.

### `GitRepoSource` — `IndexSource`

Default. Mirrors a remote git repo to the VPS index volume via `git clone --mirror` /
`git remote update`, then calls `zoekt-git-index` on the mirror. Handles bare repos.

### `FilesystemSource` — `IndexSource`

Optional. Glob-expands a local path, stages files to a temp dir, then rsyncs to the VPS
index volume via SSH (using the `tailbox` host alias), then triggers `zoekt-index` on the
VPS-side staged path. Used for `~/dev/*/docs` patterns.

### `LocalZoektSource` — `IndexSource` + `SearchProvider`

Optional sidecar. Runs `zoekt-webserver` locally on the Mac, indexes a path that stays
local (e.g. Obsidian vault when `source = "local"`). Results are merged into the
`MergedAdapter` fan-out.

### `ZoektServiceAdapter` — `ServiceManager`

SSHes into the VPS via `tailbox` to manage the Zoekt process. Runs
`zoekt-indexserver` (auto-indexes on push/schedule) and `zoekt-webserver`. Uses a
minibox container on the VPS for process isolation.

---

## Crate Layout

```
crates/searchbox/
  Cargo.toml
  src/
    lib.rs
    domain.rs          — ports, domain types, errors
    config.rs          — SearchboxConfig (serde, from TOML)
    mcp.rs             — MCP stdio tool handlers
    adapters/
      zoekt.rs         — ZoektAdapter
      merged.rs        — MergedAdapter
      git_source.rs    — GitRepoSource
      fs_source.rs     — FilesystemSource
      local.rs         — LocalZoektSource
      service.rs       — ZoektServiceAdapter
  bin/
    searchboxd.rs      — composition root: wire config → adapters → MCP stdio loop
  tests/
    search_unit.rs     — domain logic, mock adapters
    search_integration.rs  — ZoektAdapter against a live zoekt-webserver (feature-gated)
```

---

## Configuration

`~/.config/searchbox/config.toml` (or `SEARCHBOX_CONFIG` env override):

```toml
[service]
vps_host = "minibox"        # Tailscale SSH host alias
zoekt_port = 6070
index_schedule = "0 * * * *"   # cron: hourly reindex

[[repos]]
name = "minibox"
url = "git@github.com:89jobrien/minibox.git"
source = "git"              # default

[[repos]]
name = "obsidian-vault"
path = "~/Documents/Obsidian Vault"
source = "git"              # vault must be a git repo; falls back to "fs" if not

[[repos]]
name = "dev-docs"
path = "~/dev/*/docs"
source = "fs"               # glob-expanded, rsynced to VPS

# Optional local sidecar (indexes repos that stay on the Mac)
[local]
enabled = false
port = 6071
repos = []
```

---

## MCP Integration

`searchboxd` runs as a **local MCP stdio server** registered in `~/.claude/settings.json`:

```json
{
  "mcpServers": {
    "searchbox": {
      "type": "stdio",
      "command": "searchboxd",
      "args": ["--mcp"]
    }
  }
}
```

This replaces the failing `${SOURCEGRAPH_ENDPOINT}` HTTP MCP. The sourcegraph plugin can be
uninstalled once `searchboxd` is live.

### MCP tools exposed

| Tool | Description |
|------|-------------|
| `search` | Full-text search across all indexed repos |
| `list_repos` | List indexed repos and last-index timestamps |
| `reindex` | Trigger reindex of one or all repos |
| `service_status` | Check zoekt-webserver health on VPS |

Tool schemas follow the MCP 2025-03-26 spec. `searchboxd --mcp` enters a JSON-RPC stdio
loop; `searchboxd --reindex` and `searchboxd --status` are human-facing subcommands.

---

## Data Flow

```
Claude Code session
  → MCP tool call: search("SearchProvider trait")
  → mcp.rs: parse → SearchQuery
  → MergedAdapter.search(query)
      ├── ZoektAdapter → HTTP GET vps:6070/search?q=... → Vec<SearchResult>
      └── LocalZoektSource (if enabled) → HTTP GET localhost:6071/search?q=...
  → merge by score, deduplicate (repo, file, line)
  → Vec<SearchResult> serialised to MCP response JSON
```

---

## VPS Provisioning

`ZoektServiceAdapter::start()` via SSH:

1. Pull `sourcegraph/zoekt` image via minibox on VPS
2. Run container with index volume mounted at `/data/index`
3. Start `zoekt-indexserver` (watches `/data/index`, auto-indexes on git push)
4. Start `zoekt-webserver` on port 6070 (Tailscale-only, not public)
5. Register a cron via `ZoektServiceAdapter::schedule_reindex()`

Initial index populated by running `GitRepoSource::sync()` for each configured repo,
which mirrors repos into the index volume then calls `zoekt-git-index`.

---

## Testing Strategy

**Unit tests** (`search_unit.rs`):
- `MockSearchProvider` for `MergedAdapter` fan-out and dedup logic
- Config parsing roundtrips
- `SearchQuery` / `SearchResult` domain type invariants

**Integration tests** (`search_integration.rs`, feature `integration-tests`):
- Spin up a real `zoekt-webserver` in a minibox container locally
- Index a small fixture repo
- Assert search results round-trip correctly

**Manual smoke test:**
```bash
searchboxd --status          # check VPS zoekt health
searchboxd --reindex minibox # trigger single-repo reindex
```

---

## Dependencies (new)

| Crate | Purpose |
|-------|---------|
| `reqwest` | ZoektAdapter HTTP client |
| `serde_json` | Zoekt API + MCP JSON |
| `tokio` | async runtime (workspace dep) |
| `async-trait` | async trait bounds (workspace dep) |
| `futures` | `join_all` for MergedAdapter |
| `clap` | `searchboxd` CLI |
| `thiserror` | domain error types (workspace dep) |
| `tracing` | structured logging (workspace dep) |

No new workspace-level deps — all either already in `Cargo.toml` or small additions.

---

## Phase B Hook

When Gitea + Zoekt is ready (Phase B), `ZoektAdapter` is swapped for a `GiteaAdapter`
implementing the same `SearchProvider` port. No other code changes. `searchboxd` stays as
the MCP server — only its backend wiring in the composition root changes.

---

[zoekt]: https://github.com/sourcegraph/zoekt
