# searchbox + zoektbox

**Date:** 2026-04-22  **Status:** Draft  **Crates:** `crates/searchbox`, `crates/zoektbox`

Two crates, split by responsibility:

- **`zoektbox`** — owns the Zoekt binary lifecycle: pinned-release download, SHA256
  verification, upload to VPS, version management. No search logic. Follows the `tailbox`
  pattern for third-party binary provisioning.
- **`searchbox`** — owns search: `SearchProvider` port, adapters, index sources, MCP stdio
  server. Depends on `zoektbox` via the `ServiceManager` port for Zoekt process lifecycle.

Query surface: full-text code search, git-history traversal, and plaintext doc search
(Obsidian vault, `~/dev/*/docs`). Replaces the failing `${SOURCEGRAPH_ENDPOINT}` MCP plugin.

---

## Ports

```rust
#[async_trait]
pub trait SearchProvider: Send + Sync {
    async fn search(&self, query: SearchQuery) -> Result<Vec<SearchResult>, SearchError>;
    async fn list_repos(&self) -> Result<Vec<RepoInfo>, SearchError>;
}

#[async_trait]
pub trait IndexSource: Send + Sync {
    fn name(&self) -> &str;
    fn source_type(&self) -> SourceType;
    /// Sync corpus to `dest` (local staging dir) ready for zoekt-*-index.
    async fn sync(&self, dest: &Path) -> Result<SyncStats, IndexError>;
}

#[async_trait]
pub trait ServiceManager: Send + Sync {
    async fn start(&self)                            -> Result<(), ServiceError>;
    async fn stop(&self)                             -> Result<(), ServiceError>;
    async fn status(&self)                           -> Result<ServiceStatus, ServiceError>;
    async fn reindex(&self, repo: Option<&str>)      -> Result<(), ServiceError>;
}
```

---

## Domain Types

```rust
pub struct SearchQuery {
    pub text: String,
    pub repos: Option<Vec<String>>,  // None = all
    pub lang: Option<String>,
    pub case_sensitive: bool,
    pub context_lines: u8,           // default 2
}

pub struct SearchResult {
    pub repo: String,
    pub file: String,
    pub line: u32,
    pub col: u32,
    pub snippet: String,             // context_lines above + match + context_lines below
    pub score: f32,
    pub commit: Option<String>,      // SHA if result came from git history
}

pub struct RepoInfo {
    pub name: String,
    pub source_type: SourceType,
    pub last_indexed: Option<DateTime<Utc>>,
    pub doc_count: u64,
}

pub enum SourceType    { Git, Filesystem, Local }
pub enum ServiceStatus { Running, Stopped, Indexing }

#[derive(Debug, Error)]
pub enum SearchError {
    #[error("zoekt unavailable: {0}")]        Unavailable(String),
    #[error("query failed: {0}")]             QueryFailed(String),
}
#[derive(Debug, Error)]
pub enum IndexError {
    #[error("sync failed for {repo}: {reason}")] SyncFailed { repo: String, reason: String },
    #[error("index command failed: {0}")]         IndexCmd(String),
}
#[derive(Debug, Error)]
pub enum ServiceError {
    #[error("ssh: {0}")]     Ssh(String),
    #[error("process: {0}")] Process(String),
}
```

---

## Adapters

| Adapter | Trait(s) | Notes |
|---------|----------|-------|
| `ZoektAdapter` | `SearchProvider` | HTTP client to `zoekt-webserver` on VPS port 6070. Maps `SearchQuery` → Zoekt JSON API. Stateless — no knowledge of how repos were indexed. |
| `MergedAdapter` | `SearchProvider` | Fan-out over `Vec<Box<dyn SearchProvider>>` via `futures::join_all`. Merges by score, deduplicates on `(repo, file, line)`. |
| `GitRepoSource` | `IndexSource` | `git clone --mirror` / `git remote update` into the VPS index volume, then `zoekt-git-index`. Indexes all branches; history traversal comes for free. Default source type. |
| `FilesystemSource` | `IndexSource` | Glob-expands a local path, rsyncs to VPS via `tailbox` SSH host, triggers `zoekt-index` on the VPS-side path. For `~/dev/*/docs`, markdown files, etc. |
| `LocalZoektSource` | `IndexSource` + `SearchProvider` | Optional Mac sidecar. Runs `zoekt-webserver` on localhost:6071, indexes paths that must not leave the Mac (e.g. Obsidian vault when vault is not a git repo). Results merged via `MergedAdapter`. |
| `ZoektServiceAdapter` | `ServiceManager` | Implemented in `zoektbox`. SSHes into VPS via `tailbox`. Downloads pinned Zoekt release tarball, verifies SHA256, uploads binaries, runs `zoekt-indexserver` + `zoekt-webserver` inside a minibox container with `/data/index` volume. Registers cron for scheduled reindex. Version pin in `zoektbox/src/release.rs`. |

**Zoekt git history note:** `zoekt-git-index -index_all_branches` indexes every branch's
HEAD. For commit-level history search, `zoekt-git-index -branches` with explicit refs. The
`SearchResult.commit` field is populated from Zoekt's `LineMatches[].FileName` context when
the result originated from a non-HEAD ref.

---

## Crate Layout

```
crates/zoektbox/
  Cargo.toml
  src/
    lib.rs
    release.rs           — pinned version, download URL, SHA256 manifest per platform
    download.rs          — fetch tarball, verify hash, extract binaries to staging dir
    deploy.rs            — rsync binaries to VPS via tailbox SSH
    service.rs           — ZoektServiceAdapter: impl ServiceManager (start/stop/status/reindex)
  tests/
    unit.rs              — SHA256 verify, release URL construction, version parse

crates/searchbox/
  Cargo.toml             — depends on zoektbox
  src/
    lib.rs
    domain.rs            — SearchProvider, IndexSource, ServiceManager ports; all domain types + errors
    config.rs            — SearchboxConfig (serde_derive, TOML)
    mcp.rs               — JSON-RPC stdio loop; tool dispatch
    adapters/
      zoekt.rs           — ZoektAdapter (reqwest)
      merged.rs          — MergedAdapter
      git_source.rs      — GitRepoSource
      fs_source.rs       — FilesystemSource
      local.rs           — LocalZoektSource
  bin/
    searchboxd.rs        — composition root; clap subcommands: --mcp | --reindex | --status | --provision
  tests/
    unit.rs              — MergedAdapter dedup, config parse, domain invariants
    integration.rs       — ZoektAdapter against live zoekt-webserver (feature = "integration-tests")
```

`ServiceManager` port is defined in `searchbox::domain` and implemented in `zoektbox::service`.
`searchbox` depends on `zoektbox`; `zoektbox` has zero knowledge of search.

---

## Configuration

`~/.config/searchbox/config.toml` (override: `SEARCHBOX_CONFIG`):

```toml
[service]
vps_host      = "minibox"       # Tailscale SSH alias
zoekt_port    = 6070
index_schedule = "0 * * * *"   # cron; empty = manual only

[[repos]]
name   = "minibox"
url    = "git@github.com:89jobrien/minibox.git"
source = "git"

[[repos]]
name   = "obsidian-vault"
path   = "~/Documents/Obsidian Vault"
source = "git"                  # requires vault to be a git repo

[[repos]]
name   = "dev-docs"
path   = "~/dev/*/docs"
source = "fs"                   # glob-expanded; rsynced to VPS

[local]
enabled = false
port    = 6071
repos   = []                    # names from [[repos]] to index locally instead
```

`source = "git"` on a non-git path is a config error at startup (not a silent fallback).
Set `source = "fs"` explicitly for non-git corpora.

---

## MCP Integration

`searchboxd --mcp` runs a JSON-RPC 2.0 stdio loop (MCP 2025-03-26). Register in
`~/.claude/settings.json`:

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

Tools:

| Tool | Input | Notes |
|------|-------|-------|
| `search` | `{q, repos?, lang?, case_sensitive?, context_lines?}` | Returns ranked `SearchResult[]` |
| `list_repos` | — | Returns `RepoInfo[]` with last-indexed timestamps and doc counts |
| `reindex` | `{repo?}` | `repo` omitted = all; async, returns immediately |
| `service_status` | — | Checks zoekt-webserver HTTP health on VPS |

Uninstall the sourcegraph plugin after `searchboxd` is confirmed working.

---

## Data Flow

```
MCP tool call: search({q: "SearchProvider trait"})
  → mcp.rs → SearchQuery
  → MergedAdapter.search(query)
      ├── ZoektAdapter  → GET http://minibox:6070/search?q=...   → ranked results
      └── LocalZoektSource (if enabled) → GET http://localhost:6071/search?q=...
  → merge by score, dedup on (repo, file, line)
  → JSON-RPC response: SearchResult[]
```

---

## VPS Provisioning (`ZoektServiceAdapter::start`)

1. Pull `sourcegraph/zoekt` via minibox on VPS
2. Run container: index volume at `/data/index`, port 6070 bound to Tailscale IP only
3. Start `zoekt-indexserver` — watches `/data/index/*.git`, auto-reindexes on change
4. Start `zoekt-webserver -index /data/index -listen :6070`
5. Schedule cron via SSH for periodic `zoekt-git-index` sweep

Initial populate: `GitRepoSource::sync()` per configured repo mirrors into `/data/index`,
then `zoekt-git-index -index /data/index /data/index/<repo>.git`.

**Security:** port 6070 bound to Tailscale interface only. No public exposure. `zoekt-webserver`
has no auth — Tailscale ACLs are the only gate. Do not bind to `0.0.0.0`.

---

## Testing

**Unit** (`tests/unit.rs`):
- `MergedAdapter`: mock providers returning overlapping results → verify dedup + score order
- Config: valid/invalid TOML, `source = "git"` on non-git path rejects at parse
- `SearchQuery` defaults, `SearchResult` ordering

**Integration** (`tests/integration.rs`, `--features integration-tests`):
- Spawn `zoekt-webserver` in minibox container with a fixture bare repo
- `GitRepoSource::sync()` → index → `ZoektAdapter::search()` → assert hit count and snippet

**Smoke:**
```bash
searchboxd --status          # HTTP health check against VPS zoekt-webserver
searchboxd --reindex minibox # single-repo reindex, tail zoekt-indexserver log
```

---

## Dependencies

New additions only (workspace deps reused):

| Crate | Crate(s) | Use |
|-------|----------|-----|
| `reqwest` | searchbox | `ZoektAdapter` HTTP client |
| `futures` | searchbox | `join_all` in `MergedAdapter` |
| `clap` | searchbox | `searchboxd` CLI |
| `serde_json` | searchbox | Zoekt API + MCP wire format |
| `sha2` | zoektbox | release tarball verification |
| `hex` | zoektbox | SHA256 hex encoding (already in minibox-secrets) |
| `flate2` + `tar` | zoektbox | tarball extraction |

`tokio`, `async-trait`, `thiserror`, `tracing`, `serde` — workspace deps, no version bump needed.

---

## Phase B

`GiteaAdapter` implements `SearchProvider` against the Gitea API. Swap at the composition
root in `searchboxd.rs` — no other files change. `searchboxd` MCP surface is identical.

[zoekt]: https://github.com/sourcegraph/zoekt
