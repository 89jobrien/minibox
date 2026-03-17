# Useful Patterns from Zombienet-SDK

**Source:** ~/dev/zombienet-sdk-main/
**Project:** ZombieNet SDK - Testing framework for blockchain networks
**Architecture:** Provider pattern with native/docker/kubernetes backends

## Overview

Zombienet-SDK is a Rust testing framework for blockchain networks that uses a provider abstraction pattern identical to our hexagonal architecture. Their implementation validates many of our design choices and provides additional patterns we can adopt.

## Key Architectural Similarities

### 1. Provider Trait Pattern (Identical to Our Adapters)

**Their Pattern:**

```rust
pub type DynProvider = Arc<dyn Provider + Send + Sync>;
pub type DynNamespace = Arc<dyn ProviderNamespace + Send + Sync>;
pub type DynNode = Arc<dyn ProviderNode + Send + Sync>;

#[async_trait]
pub trait Provider {
    fn name(&self) -> &str;
    fn capabilities(&self) -> &ProviderCapabilities;
    async fn namespaces(&self) -> HashMap<String, DynNamespace>;
    async fn create_namespace(&self) -> Result<DynNamespace, ProviderError>;
}
```

**Our Pattern:**

```rust
pub trait ImageRegistry: AsAny + Send + Sync {
    async fn has_image(&self, name: &str, tag: &str) -> bool;
    async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata>;
}

// Usage with Arc<dyn Trait>
Arc<dyn ImageRegistry>
```

**Similarity:**

- Both use `Arc<dyn Trait + Send + Sync>` for dynamic dispatch
- Both use `async_trait` for async methods
- Both define type aliases for ergonomics (`DynProvider` vs our usage of `Arc<dyn Trait>`)

**What We Can Adopt:**

```rust
// Add type aliases for cleaner API
pub type DynImageRegistry = Arc<dyn ImageRegistry + Send + Sync>;
pub type DynFilesystemProvider = Arc<dyn FilesystemProvider + Send + Sync>;
pub type DynResourceLimiter = Arc<dyn ResourceLimiter + Send + Sync>;
pub type DynContainerRuntime = Arc<dyn ContainerRuntime + Send + Sync>;
```

### 2. Comprehensive Error Enum with thiserror

**Their Pattern:**

```rust
#[derive(Debug, thiserror::Error)]
pub enum ProviderError {
    #[error("Failed to create namespace '{0}': {1}")]
    CreateNamespaceFailed(String, anyhow::Error),

    #[error("Failed to spawn node '{0}': {1}")]
    NodeSpawningFailed(String, anyhow::Error),

    #[error("Error running command '{0}' {1}: {2}")]
    RunCommandError(String, String, anyhow::Error),

    #[error("Invalid network configuration field {0}")]
    InvalidConfig(String),

    #[error(transparent)]
    FileSystemError(#[from] FileSystemError),
}
```

**Benefits:**

- Structured errors with context (container ID, command, etc.)
- Wrapped `anyhow::Error` for underlying causes
- `#[from]` for automatic error conversion
- Rich error messages with interpolation

**What We Can Adopt:**

```rust
// Extend our DomainError enum with more context
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("Failed to spawn container '{id}': {source}")]
    ContainerSpawnFailed {
        id: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Failed to pull image '{image}:{tag}': {source}")]
    ImagePullFailed {
        image: String,
        tag: String,
        #[source]
        source: anyhow::Error,
    },

    // Add transparent wrapper for infrastructure errors
    #[error(transparent)]
    InfrastructureError(#[from] anyhow::Error),
}
```

### 3. Capability-Based Design

**Their Pattern:**

```rust
pub struct ProviderCapabilities {
    requires_image: bool,
    has_resources: bool,
    prefix_with_full_path: bool,
    use_default_ports_in_cmd: bool,
}

impl Provider {
    fn capabilities(&self) -> &ProviderCapabilities;
}
```

**What This Enables:**

- Runtime feature detection
- Conditional behavior based on provider capabilities
- Clear documentation of what each provider supports

**What We Can Adopt:**

```rust
pub struct RuntimeCapabilities {
    pub supports_user_namespaces: bool,
    pub supports_cgroups_v2: bool,
    pub supports_overlay_fs: bool,
    pub supports_network_isolation: bool,
    pub max_containers: Option<usize>,
}

impl ContainerRuntime {
    fn capabilities(&self) -> &RuntimeCapabilities;
}

// Usage example
if runtime.capabilities().supports_user_namespaces {
    // Use user namespace remapping
} else {
    // Fall back to rootful containers
}
```

### 4. Hierarchical Resource Model

**Their Pattern:**

```
Provider
  └─ Namespace (workspace isolation)
      └─ Node (individual process)
```

**Our Potential Hierarchy:**

```
ContainerRuntime
  └─ Namespace (future: multi-container networks)
      └─ Container (individual process)
```

**Benefits:**

- Logical grouping of related containers
- Namespace-level resource limits
- Easier cleanup (destroy namespace destroys all nodes)

### 5. JSON-Based Serialization for State

**Their Pattern:**

```rust
#[async_trait]
pub trait ProviderNode: erased_serde::Serialize {
    // Node can be serialized to JSON for persistence
}

async fn create_namespace_from_json(
    &self,
    json_value: &serde_json::Value,
) -> Result<DynNamespace, ProviderError>;
```

**Benefits:**

- Easy state persistence
- Configuration as code
- Network topology as JSON

**What We Can Adopt:**

```rust
use erased_serde::Serialize;

pub trait Container: Serialize + Send + Sync {
    fn id(&self) -> &str;
    fn state(&self) -> ContainerState;
    // ... other methods
}

// Enable container state serialization for persistence
pub struct DaemonState {
    containers: HashMap<String, Box<dyn Container>>,
}

impl DaemonState {
    pub fn save_to_disk(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(&self.containers)?;
        std::fs::write(path, json)?;
        Ok(())
    }
}
```

### 6. Shared Types Module

**Their Structure:**

```
crates/provider/src/
├── lib.rs              # Main traits
├── shared/
│   ├── types.rs        # Shared types across all providers
│   ├── constants.rs    # Shared constants
│   └── helpers.rs      # Shared utility functions
├── docker/             # Docker provider implementation
├── native/             # Native provider implementation
└── kubernetes/         # Kubernetes provider implementation
```

**Benefits:**

- Reduces duplication
- Ensures consistency across providers
- Clear separation of concerns

**What We Can Adopt:**

```
crates/minibox-lib/src/
├── domain.rs
├── adapters/
│   ├── mod.rs
│   ├── shared/
│   │   ├── types.rs        # Common types (ResourceConfig, etc.)
│   │   ├── constants.rs    # Shared constants
│   │   └── helpers.rs      # Path validation, etc.
│   ├── registry.rs         # DockerHub adapter
│   ├── colima.rs           # Colima adapter
│   ├── wsl.rs              # WSL adapter
│   └── docker_desktop.rs   # Docker Desktop adapter
```

### 7. File Server Pattern

**Their Pattern:**

```
crates/file-server/     # Standalone HTTP file server
```

They use a dedicated file server for serving files to containers.

**What We Can Adopt:**

- Add HTTP file server for serving files into containers
- Useful for configuration injection without volume mounts
- Enables secure file distribution to isolated containers

## Code Structure Patterns

### Workspace Organization

**Their Cargo.toml:**

```toml
[workspace]
members = [
    "crates/cli",
    "crates/configuration",
    "crates/orchestrator",
    "crates/provider",
    "crates/sdk",
    "crates/support",
    "crates/test-runner",
]

[workspace.package]
edition = "2021"
version = "0.0.0"
license = "MIT"
```

**Our Cargo.toml (already similar):**

```toml
[workspace]
members = [
    "crates/minibox-lib",
    "crates/miniboxd",
    "crates/minibox-cli",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"
```

Both use workspace-level package configuration for consistency.

### Async-First Design

**Their Pattern:**
All provider operations are async, even when underlying implementation might be sync. This provides:

- Consistent API across providers
- Future-proofing for async implementations
- Better resource utilization

**What We Already Do:**

```rust
#[async_trait]
pub trait ImageRegistry {
    async fn pull_image(&self, name: &str, tag: &str) -> Result<ImageMetadata>;
}
```

We already follow this pattern!

## Testing Patterns

### Provider-Agnostic Tests

**Their Approach:**

- Tests use `DynProvider` trait objects
- Same tests run against native, docker, and kubernetes providers
- Validates behavioral parity

**Our Approach (already implemented):**

- Conformance tests use mock implementations
- Tests validate all adapters behave identically
- See `crates/miniboxd/tests/conformance_tests.rs`

## Recommended Adoptions for Minibox

### Priority 1: Type Aliases (Low Effort, High Clarity)

```rust
// In domain.rs
pub type DynImageRegistry = Arc<dyn ImageRegistry + Send + Sync>;
pub type DynFilesystemProvider = Arc<dyn FilesystemProvider + Send + Sync>;
pub type DynResourceLimiter = Arc<dyn ResourceLimiter + Send + Sync>;
pub type DynContainerRuntime = Arc<dyn ContainerRuntime + Send + Sync>;

// In handler.rs
pub struct HandlerDependencies {
    pub registry: DynImageRegistry,
    pub filesystem: DynFilesystemProvider,
    pub resource_limiter: DynResourceLimiter,
    pub runtime: DynContainerRuntime,
}
```

**Benefits:**

- Cleaner API
- Easier to read signatures
- Matches industry standard (zombienet, kubernetes-rs, etc.)

### Priority 2: Enhanced Error Types (Medium Effort, High Impact)

```rust
#[derive(Debug, thiserror::Error)]
pub enum DomainError {
    #[error("Container '{id}' failed to spawn: {source}")]
    ContainerSpawnFailed {
        id: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Image '{image}:{tag}' pull failed: {source}")]
    ImagePullFailed {
        image: String,
        tag: String,
        #[source]
        source: anyhow::Error,
    },

    #[error("Resource limit '{limit}' exceeded: {value} > {max}")]
    ResourceLimitExceeded {
        limit: String,
        value: u64,
        max: u64,
    },

    #[error(transparent)]
    InfrastructureError(#[from] anyhow::Error),
}
```

**Benefits:**

- Better error messages with context
- Easier debugging
- Structured error data for logging

### Priority 3: Capability Detection (Medium Effort, Future-Proofing)

```rust
pub struct RuntimeCapabilities {
    pub supports_user_namespaces: bool,
    pub supports_cgroups_v2: bool,
    pub supports_overlay_fs: bool,
    pub supports_network_isolation: bool,
}

pub trait ContainerRuntime {
    fn capabilities(&self) -> RuntimeCapabilities;
    // ... other methods
}
```

**Benefits:**

- Runtime feature detection
- Graceful degradation
- Clear documentation of provider limitations

### Priority 4: State Serialization (High Effort, High Value)

```rust
pub trait Container: erased_serde::Serialize + Send + Sync {
    // Containers can be serialized for persistence
}

impl DaemonState {
    pub fn save(&self, path: &Path) -> Result<()>;
    pub fn load(path: &Path) -> Result<Self>;
}
```

**Benefits:**

- Daemon restart doesn't lose state
- State can be inspected/debugged
- Enables migration/backup

## Files to Study

### Most Relevant for Minibox

1. **Provider Trait Pattern:**
   - `~/dev/zombienet-sdk-main/crates/provider/src/lib.rs`
   - Clean trait design with error handling

2. **Docker Provider Implementation:**
   - `~/dev/zombienet-sdk-main/crates/provider/src/docker/provider.rs`
   - Shows how to delegate to Docker API

3. **Native Provider Implementation:**
   - `~/dev/zombienet-sdk-main/crates/provider/src/native/provider.rs`
   - Shows native Linux implementation

4. **Shared Types:**
   - `~/dev/zombienet-sdk-main/crates/provider/src/shared/types.rs`
   - Common types across providers

## Key Takeaways

1. **Our architecture is validated** - Zombienet uses identical pattern (trait objects with Arc)
2. **Type aliases improve ergonomics** - `DynProvider` pattern worth adopting
3. **Structured errors are powerful** - thiserror with context fields
4. **Capability detection is valuable** - Runtime feature queries
5. **State serialization is critical** - erased_serde enables persistence

## Conclusion

Zombienet-SDK validates our hexagonal architecture approach and provides proven patterns for:

- Error handling (structured thiserror enums)
- Type ergonomics (type aliases for Arc<dyn Trait>)
- Capability detection (runtime feature queries)
- State persistence (erased_serde)

These patterns can be incrementally adopted to improve minibox's API clarity and functionality.

---

**Analysis Date:** 2026-03-16
**Zombienet Version:** Examined from ~/dev/zombienet-sdk-main/
**Minibox Version:** 0.1.0 (post-security framework)
