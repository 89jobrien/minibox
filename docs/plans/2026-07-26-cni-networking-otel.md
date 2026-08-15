# Plan: minibox-cni — CNI-spec bridge networking + OTEL span instrumentation

## Goal

Implement `minibox-cni`, a new crate providing CNI-spec-compliant plugin exec/chain
orchestration, wire it in as a `minibox-core::domain::NetworkProvider` adapter for the native
Linux suite behind a new `cni` feature flag, and add `#[instrument]` OTEL spans across the daemon
request boundary, the native runtime's spawn path, and the new CNI chain — per the approved
design at `docs/designs/2026-07-26-cni-networking-otel-design.md`.

## Architecture

- **Crates affected**: `minibox-cni` (new), `minibox` (feature flag, `#[instrument]` on
  `handle_run`/`spawn_process`), `miniboxd` (feature flag forwarding, `resolve_native_network()`
  swap), `mbx` (doctor preflight check).
- **New traits/types**: none new — `minibox-cni::provider::CniNetworkProvider` implements the
  **existing** `minibox_core::domain::NetworkProvider` port. New types are `NetworkConfigList`,
  `PluginConfig`, `CniResult`, `CniInterface`, `CniIpConfig`, `CniDns`, `CniErrorPayload`,
  `CniError` — all in `minibox-cni`.
- **Data flow**: `mbx run` → `handle_run` (`crates/minibox/src/daemon/handler/run.rs:129`) →
  `LinuxNamespaceRuntime::spawn_process` (`crates/minibox/src/adapters/runtime.rs:109`, creates
  netns) → native suite's `NetworkProvider::setup`/`attach` (now `CniNetworkProvider`) →
  `NetworkConfigList::add` (`minibox-cni`) → per-plugin `tokio::process::Command` exec → merged
  `CniResult` recorded on the container; teardown reverses via `cleanup`/`NetworkConfigList::del`.

## Tech Stack

- Rust edition 2024, `tokio` (async process spawn), `serde`/`serde_json` (CNI JSON protocol),
  `thiserror` + `miette` (error type), `tracing` (`#[instrument]`), `async-trait` (port impl),
  `dashmap` (netns-by-container tracking in `CniNetworkProvider` — already a workspace dep, no
  new external dependency introduced).

## Tasks

### Task 1: Scaffold `minibox-cni` crate

**Crate**: `minibox-cni`
**File(s)**: `Cargo.toml`, `crates/minibox-cni/Cargo.toml`, `crates/minibox-cni/src/lib.rs`
**Run**: `cargo check -p minibox-cni`

1. Add to root `Cargo.toml` `[workspace.members]` (after `"crates/mcp",`):
   ```toml
       "crates/minibox-cni",
   ```
   Add to root `Cargo.toml` `[workspace.dependencies]` (alphabetical, near other `minibox-*`
   entries):
   ```toml
   minibox-cni = { path = "crates/minibox-cni" }
   ```

2. Create `crates/minibox-cni/Cargo.toml`:
   ```toml
   [package]
   name = "minibox-cni"
   version.workspace = true
   edition.workspace = true
   license.workspace = true
   rust-version.workspace = true
   repository.workspace = true
   description = "CNI (Container Network Interface) plugin exec protocol and chain orchestration"
   publish = false

   [dependencies]
   minibox-core = { workspace = true }
   anyhow = { workspace = true }
   async-trait = { workspace = true }
   dashmap = { workspace = true }
   miette = { workspace = true }
   serde = { workspace = true }
   serde_json = { workspace = true }
   thiserror = { workspace = true }
   tokio = { workspace = true }
   tracing = { workspace = true }

   [dev-dependencies]
   tempfile = { workspace = true }

   [lints]
   workspace = true
   ```

3. Create `crates/minibox-cni/src/lib.rs`:
   ```rust
   //! CNI (Container Network Interface) plugin exec protocol and chain
   //! orchestration for minibox's native Linux adapter.
   //!
   //! This crate is deliberately ignorant of how a network namespace is
   //! obtained — callers pass an opaque `netns: &str` target straight through
   //! to the `CNI_NETNS` environment variable. On Linux that's a
   //! `/proc/<pid>/ns/net` path; nothing in this crate assumes that format,
   //! keeping the door open for a future non-Linux (WinCNI/HNS) caller
   //! without modification here.

   pub mod config;
   pub mod error;
   pub mod exec;
   pub mod provider;
   pub mod result;

   pub use config::{NetworkConfigList, PluginConfig};
   pub use error::CniError;
   pub use provider::CniNetworkProvider;
   pub use result::{CniDns, CniErrorPayload, CniInterface, CniIpConfig, CniResult};
   ```

4. Create empty placeholder modules so `cargo check` passes (bodies filled in later tasks):
   `crates/minibox-cni/src/config.rs`, `crates/minibox-cni/src/error.rs`,
   `crates/minibox-cni/src/exec.rs`, `crates/minibox-cni/src/provider.rs`,
   `crates/minibox-cni/src/result.rs` — each starting with just a module doc comment, e.g.
   `crates/minibox-cni/src/error.rs`:
   ```rust
   //! Errors from CNI plugin execution.
   ```
   (same one-line pattern for the other four files, module purpose in the doc comment).

5. Verify:
   ```
   cargo check -p minibox-cni    → compiles (empty modules)
   ```

6. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox-cni): scaffold crate"`

---

### Task 2: `CniError` type

**Crate**: `minibox-cni`
**File(s)**: `crates/minibox-cni/src/error.rs`
**Run**: `cargo nextest run -p minibox-cni`

1. Write failing test (append to `crates/minibox-cni/src/error.rs`):
   ```rust
   #[cfg(test)]
   #[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
   mod tests {
       use super::*;

       #[test]
       fn plugin_not_found_display_names_the_plugin() {
           let err = CniError::PluginNotFound {
               plugin: "bridge".to_string(),
               searched: vec![PathBuf::from("/opt/cni/bin")],
           };
           assert!(err.to_string().contains("bridge"));
           assert!(err.to_string().contains("not found"));
       }

       #[test]
       fn plugin_error_display_includes_msg() {
           let err = CniError::PluginError {
               plugin: "host-local".to_string(),
               code: Some(7),
               msg: "no IP addresses available".to_string(),
               details: None,
           };
           assert!(err.to_string().contains("host-local"));
           assert!(err.to_string().contains("no IP addresses available"));
       }
   }
   ```
   Run: `cargo nextest run -p minibox-cni -- plugin_not_found_display_names_the_plugin`
   Expected: FAIL (`CniError` does not exist yet)

2. Implement (prepend to `crates/minibox-cni/src/error.rs`, above the `#[cfg(test)]` block):
   ```rust
   //! Errors from CNI plugin execution.

   use std::path::PathBuf;

   /// Errors from CNI plugin execution.
   #[derive(Debug, thiserror::Error, miette::Diagnostic)]
   pub enum CniError {
       /// A required plugin binary was not found on the configured `CNI_PATH`.
       #[error("CNI plugin '{plugin}' not found on CNI_PATH")]
       #[diagnostic(code(minibox::cni::plugin_not_found))]
       PluginNotFound {
           /// The plugin type that was requested (e.g. `"bridge"`).
           plugin: String,
           /// The directories that were searched.
           searched: Vec<PathBuf>,
       },

       /// A plugin returned a CNI-spec structured error object on stdout.
       #[error("CNI plugin '{plugin}' failed: {msg}")]
       #[diagnostic(code(minibox::cni::plugin_error))]
       PluginError {
           /// The plugin type that failed.
           plugin: String,
           /// The CNI spec error code, if the plugin reported one.
           code: Option<u32>,
           /// Human-readable error message from the plugin.
           msg: String,
           /// Optional additional detail from the plugin.
           details: Option<String>,
       },

       /// A plugin process exited non-zero (or was signal-killed) without a
       /// structured CNI error object on stdout.
       #[error("CNI plugin '{plugin}' exited with status {exit_code:?}")]
       #[diagnostic(code(minibox::cni::process_failed))]
       ProcessFailed {
           /// The plugin type that failed.
           plugin: String,
           /// Process exit code, or `None` if signal-killed.
           exit_code: Option<i32>,
           /// Captured stderr output.
           stderr: String,
       },

       /// Failed to parse a `.conflist` file or a plugin's JSON payload.
       #[error("failed to parse CNI JSON: {0}")]
       #[diagnostic(code(minibox::cni::config_parse))]
       ConfigParse(#[from] serde_json::Error),

       /// I/O error spawning or communicating with a plugin process.
       #[error("CNI plugin I/O error: {0}")]
       #[diagnostic(code(minibox::cni::io))]
       Io(#[from] std::io::Error),
   }
   ```

3. Verify:
   ```
   cargo nextest run -p minibox-cni    → all green
   cargo clippy -p minibox-cni -- -D warnings    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox-cni): add CniError type"`

---

### Task 3: Result types (`CniResult`, `CniInterface`, `CniIpConfig`, `CniDns`, `CniErrorPayload`)

**Crate**: `minibox-cni`
**File(s)**: `crates/minibox-cni/src/result.rs`
**Run**: `cargo nextest run -p minibox-cni`

1. Write failing test (append to `crates/minibox-cni/src/result.rs`):
   ```rust
   #[cfg(test)]
   #[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
   mod tests {
       use super::*;

       #[test]
       fn cni_result_deserializes_add_output() {
           let json = r#"{
               "cniVersion": "1.0.0",
               "interfaces": [{"name": "eth0", "mac": "aa:bb:cc:dd:ee:ff"}],
               "ips": [{"address": "10.88.0.5/24", "gateway": "10.88.0.1", "interface": 0}],
               "dns": {"nameservers": ["10.88.0.1"]}
           }"#;
           let result: CniResult = serde_json::from_str(json).expect("deserialize");
           assert_eq!(result.cni_version, "1.0.0");
           assert_eq!(result.interfaces[0].name, "eth0");
           assert_eq!(result.ips[0].address, "10.88.0.5/24");
           assert_eq!(result.dns.nameservers, vec!["10.88.0.1".to_string()]);
       }

       #[test]
       fn cni_result_defaults_missing_optional_fields() {
           let json = r#"{"cniVersion": "1.0.0"}"#;
           let result: CniResult = serde_json::from_str(json).expect("deserialize");
           assert!(result.interfaces.is_empty());
           assert!(result.ips.is_empty());
           assert!(result.dns.nameservers.is_empty());
       }

       #[test]
       fn cni_error_payload_deserializes_spec_error() {
           let json = r#"{"code": 7, "msg": "no IPs", "details": "pool exhausted"}"#;
           let payload: CniErrorPayload = serde_json::from_str(json).expect("deserialize");
           assert_eq!(payload.code, 7);
           assert_eq!(payload.msg, "no IPs");
           assert_eq!(payload.details.as_deref(), Some("pool exhausted"));
       }
   }
   ```
   Run: `cargo nextest run -p minibox-cni -- cni_result_deserializes_add_output`
   Expected: FAIL (types do not exist yet)

2. Implement (prepend to `crates/minibox-cni/src/result.rs`):
   ```rust
   //! Result types returned by a CNI plugin chain's ADD command, and the
   //! CNI-spec structured error payload a plugin may return on failure.

   use serde::{Deserialize, Serialize};

   /// Merged result of a CNI ADD chain (the final `prevResult`).
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct CniResult {
       /// CNI spec version the result conforms to.
       #[serde(rename = "cniVersion")]
       pub cni_version: String,
       /// Network interfaces created by the chain.
       #[serde(default)]
       pub interfaces: Vec<CniInterface>,
       /// IP configurations allocated by the chain.
       #[serde(default)]
       pub ips: Vec<CniIpConfig>,
       /// DNS configuration reported by the chain.
       #[serde(default)]
       pub dns: CniDns,
   }

   /// A network interface reported by a plugin.
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct CniInterface {
       /// Interface name inside the container (e.g. `"eth0"`).
       pub name: String,
       /// MAC address, if reported.
       #[serde(default)]
       pub mac: Option<String>,
       /// Network namespace path the interface lives in, if reported.
       #[serde(default)]
       pub sandbox: Option<String>,
   }

   /// An allocated IP configuration reported by a plugin.
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct CniIpConfig {
       /// CIDR address (e.g. `"10.88.0.5/24"`).
       pub address: String,
       /// Gateway address, if reported.
       #[serde(default)]
       pub gateway: Option<String>,
       /// Index into `CniResult::interfaces` this IP belongs to.
       #[serde(default)]
       pub interface: Option<usize>,
   }

   /// DNS configuration reported by a plugin (e.g. the `dnsname` plugin).
   #[derive(Debug, Clone, Default, Serialize, Deserialize)]
   pub struct CniDns {
       /// Nameserver IPs.
       #[serde(default)]
       pub nameservers: Vec<String>,
       /// Search domain.
       #[serde(default)]
       pub domain: Option<String>,
       /// Search list.
       #[serde(default)]
       pub search: Vec<String>,
       /// Resolver options.
       #[serde(default)]
       pub options: Vec<String>,
   }

   /// A CNI-spec structured error object, as returned by a well-behaved
   /// plugin on stdout when it fails.
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct CniErrorPayload {
       /// CNI spec error code.
       pub code: u32,
       /// Human-readable error message.
       pub msg: String,
       /// Optional additional detail.
       #[serde(default)]
       pub details: Option<String>,
   }
   ```

3. Verify:
   ```
   cargo nextest run -p minibox-cni    → all green
   cargo clippy -p minibox-cni -- -D warnings    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox-cni): add CNI result and error-payload types"`

---

### Task 4: `NetworkConfigList`/`PluginConfig` parsing

**Crate**: `minibox-cni`
**File(s)**: `crates/minibox-cni/src/config.rs`
**Run**: `cargo nextest run -p minibox-cni`

1. Write failing test (append to `crates/minibox-cni/src/config.rs`):
   ```rust
   #[cfg(test)]
   #[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
   mod tests {
       use super::*;
       use std::io::Write;

       fn write_conflist(contents: &str) -> tempfile::NamedTempFile {
           let mut file = tempfile::NamedTempFile::new().expect("create tempfile");
           file.write_all(contents.as_bytes()).expect("write conflist");
           file
       }

       #[test]
       fn from_file_parses_plugin_chain() {
           let file = write_conflist(
               r#"{
                   "cniVersion": "1.0.0",
                   "name": "minibox0",
                   "plugins": [
                       {"type": "bridge", "bridge": "minibox0"},
                       {"type": "portmap", "capabilities": {"portMappings": true}}
                   ]
               }"#,
           );
           let parsed = NetworkConfigList::from_file(file.path()).expect("parse conflist");
           assert_eq!(parsed.cni_version, "1.0.0");
           assert_eq!(parsed.name, "minibox0");
           assert_eq!(parsed.plugins.len(), 2);
           assert_eq!(parsed.plugins[0].plugin_type, "bridge");
           assert_eq!(parsed.plugins[1].plugin_type, "portmap");
       }

       #[test]
       fn from_file_rejects_malformed_json() {
           let file = write_conflist("not json");
           let result = NetworkConfigList::from_file(file.path());
           assert!(matches!(result, Err(CniError::ConfigParse(_))));
       }

       #[test]
       fn from_file_rejects_missing_file() {
           let result = NetworkConfigList::from_file(std::path::Path::new("/nonexistent/x.conflist"));
           assert!(matches!(result, Err(CniError::Io(_))));
       }
   }
   ```
   Run: `cargo nextest run -p minibox-cni -- from_file_parses_plugin_chain`
   Expected: FAIL (`NetworkConfigList` does not exist yet)

2. Implement (prepend to `crates/minibox-cni/src/config.rs`, above the `#[cfg(test)]` block):
   ```rust
   //! Parsing of CNI `.conflist` network configuration files.

   use crate::error::CniError;
   use serde::{Deserialize, Serialize};
   use std::path::Path;

   /// Parsed CNI network configuration list (`.conflist` format).
   #[derive(Debug, Clone, Deserialize)]
   pub struct NetworkConfigList {
       /// CNI spec version this config conforms to.
       #[serde(rename = "cniVersion")]
       pub cni_version: String,
       /// Network name.
       pub name: String,
       /// Ordered chain of plugins to invoke.
       pub plugins: Vec<PluginConfig>,
   }

   /// A single plugin's configuration within a `.conflist` chain.
   #[derive(Debug, Clone, Serialize, Deserialize)]
   pub struct PluginConfig {
       /// Plugin binary name (e.g. `"bridge"`, `"host-local"`).
       #[serde(rename = "type")]
       pub plugin_type: String,
       /// The plugin's own configuration fields, kept as raw JSON since each
       /// plugin defines its own schema.
       #[serde(flatten)]
       pub raw: serde_json::Value,
   }

   impl NetworkConfigList {
       /// Parse a `.conflist` file from disk.
       ///
       /// # Errors
       ///
       /// Returns [`CniError::Io`] if the file cannot be read, or
       /// [`CniError::ConfigParse`] if it is not valid CNI JSON.
       pub fn from_file(path: &Path) -> Result<Self, CniError> {
           let bytes = std::fs::read(path)?;
           let parsed = serde_json::from_slice(&bytes)?;
           Ok(parsed)
       }
   }
   ```

3. Verify:
   ```
   cargo nextest run -p minibox-cni    → all green
   cargo clippy -p minibox-cni -- -D warnings    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox-cni): add NetworkConfigList/.conflist parsing"`

---

### Task 5: `exec_plugin` — happy path

**Crate**: `minibox-cni`
**File(s)**: `crates/minibox-cni/src/exec.rs`
**Run**: `cargo nextest run -p minibox-cni`

1. Write failing test (append to `crates/minibox-cni/src/exec.rs`):
   ```rust
   #[cfg(test)]
   #[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
   mod tests {
       use super::*;
       use crate::config::PluginConfig;
       use serde_json::json;
       use std::os::unix::fs::PermissionsExt;

       /// Write an executable shell-script fixture plugin into `dir` that
       /// echoes canned JSON on ADD and exits 0 with no output on DEL.
       fn write_fixture_plugin(dir: &std::path::Path, name: &str, add_json: &str) -> std::path::PathBuf {
           let path = dir.join(name);
           let script = format!(
               "#!/bin/sh\nif [ \"$CNI_COMMAND\" = \"ADD\" ]; then\n  echo '{add_json}'\nfi\nexit 0\n"
           );
           std::fs::write(&path, script).expect("write fixture plugin");
           let mut perms = std::fs::metadata(&path).expect("stat fixture plugin").permissions();
           perms.set_mode(0o755);
           std::fs::set_permissions(&path, perms).expect("chmod fixture plugin");
           path
       }

       #[tokio::test]
       async fn exec_plugin_add_returns_parsed_json() {
           let dir = tempfile::tempdir().expect("tempdir");
           write_fixture_plugin(
               dir.path(),
               "fake-bridge",
               r#"{\"cniVersion\":\"1.0.0\",\"interfaces\":[{\"name\":\"eth0\"}]}"#,
           );
           let plugin = PluginConfig {
               plugin_type: "fake-bridge".to_string(),
               raw: json!({"type": "fake-bridge"}),
           };

           let result = exec_plugin(
               &[dir.path().to_path_buf()],
               &plugin,
               "ADD",
               "/fake/netns",
               "container-1",
               "eth0",
               None,
           )
           .await
           .expect("exec_plugin should succeed");

           assert_eq!(result["cniVersion"], "1.0.0");
           assert_eq!(result["interfaces"][0]["name"], "eth0");
       }

       #[tokio::test]
       async fn exec_plugin_missing_binary_returns_plugin_not_found() {
           let dir = tempfile::tempdir().expect("tempdir");
           let plugin = PluginConfig {
               plugin_type: "does-not-exist".to_string(),
               raw: json!({"type": "does-not-exist"}),
           };

           let result = exec_plugin(
               &[dir.path().to_path_buf()],
               &plugin,
               "ADD",
               "/fake/netns",
               "container-1",
               "eth0",
               None,
           )
           .await;

           assert!(matches!(result, Err(CniError::PluginNotFound { .. })));
       }
   }
   ```
   Run: `cargo nextest run -p minibox-cni -- exec_plugin_add_returns_parsed_json`
   Expected: FAIL (`exec_plugin` does not exist yet)

2. Implement (prepend to `crates/minibox-cni/src/exec.rs`, above the `#[cfg(test)]` block):
   ```rust
   //! CNI plugin exec protocol: spawns a plugin binary per the CNI spec,
   //! passing config over stdin and reading the JSON result from stdout.

   use crate::config::PluginConfig;
   use crate::error::CniError;
   use crate::result::CniErrorPayload;
   use std::path::PathBuf;
   use tokio::io::AsyncWriteExt;
   use tokio::process::Command;
   use tracing::{Span, instrument};

   /// Locate a plugin binary by name on the given `CNI_PATH` directories.
   fn find_plugin_binary(cni_path: &[PathBuf], plugin_type: &str) -> Result<PathBuf, CniError> {
       for dir in cni_path {
           let candidate = dir.join(plugin_type);
           if candidate.is_file() {
               return Ok(candidate);
           }
       }
       Err(CniError::PluginNotFound {
           plugin: plugin_type.to_string(),
           searched: cni_path.to_vec(),
       })
   }

   /// Invoke a single CNI plugin with the given command (`ADD`/`DEL`/`CHECK`).
   ///
   /// Returns the plugin's JSON result on success. On failure, distinguishes
   /// a CNI-spec structured error payload from a raw process failure and
   /// records both as span fields (in addition to the automatic
   /// `#[instrument(err)]` summary).
   ///
   /// # Errors
   ///
   /// Returns [`CniError::PluginNotFound`] if the plugin binary isn't on
   /// `cni_path`, [`CniError::PluginError`] if the plugin returned a CNI-spec
   /// structured error, [`CniError::ProcessFailed`] if it exited non-zero
   /// without one, or [`CniError::Io`]/[`CniError::ConfigParse`] for
   /// process-spawn or JSON errors.
   #[instrument(
       skip(cni_path, plugin, prev_result),
       fields(
           plugin = %plugin.plugin_type,
           command = %command,
           exit_code = tracing::field::Empty,
           cni_error_code = tracing::field::Empty,
           cni_error_msg = tracing::field::Empty,
           stderr = tracing::field::Empty,
       ),
       err
   )]
   pub(crate) async fn exec_plugin(
       cni_path: &[PathBuf],
       plugin: &PluginConfig,
       command: &str,
       netns: &str,
       container_id: &str,
       ifname: &str,
       prev_result: Option<&serde_json::Value>,
   ) -> Result<serde_json::Value, CniError> {
       let binary = find_plugin_binary(cni_path, &plugin.plugin_type)?;

       let mut config = plugin.raw.clone();
       if let (Some(obj), Some(prev)) = (config.as_object_mut(), prev_result) {
           obj.insert("prevResult".to_string(), prev.clone());
       }
       let config_bytes = serde_json::to_vec(&config)?;

       let cni_path_joined =
           std::env::join_paths(cni_path).map_err(|e| CniError::Io(std::io::Error::other(e)))?;

       let mut child = Command::new(&binary)
           .env("CNI_COMMAND", command)
           .env("CNI_CONTAINERID", container_id)
           .env("CNI_NETNS", netns)
           .env("CNI_IFNAME", ifname)
           .env("CNI_PATH", &cni_path_joined)
           .stdin(std::process::Stdio::piped())
           .stdout(std::process::Stdio::piped())
           .stderr(std::process::Stdio::piped())
           .spawn()?;

       let mut stdin = child.stdin.take().expect("stdin was piped above");
       stdin.write_all(&config_bytes).await?;
       drop(stdin);

       let output = child.wait_with_output().await?;
       let stderr = String::from_utf8_lossy(&output.stderr).to_string();

       if !output.status.success() {
           if let Ok(payload) = serde_json::from_slice::<CniErrorPayload>(&output.stdout) {
               Span::current().record("cni_error_code", payload.code);
               Span::current().record("cni_error_msg", payload.msg.as_str());
               Span::current().record("stderr", stderr.as_str());
               return Err(CniError::PluginError {
                   plugin: plugin.plugin_type.clone(),
                   code: Some(payload.code),
                   msg: payload.msg,
                   details: payload.details,
               });
           }

           Span::current().record("exit_code", output.status.code());
           Span::current().record("stderr", stderr.as_str());
           return Err(CniError::ProcessFailed {
               plugin: plugin.plugin_type.clone(),
               exit_code: output.status.code(),
               stderr,
           });
       }

       if output.stdout.is_empty() {
           return Ok(serde_json::Value::Null);
       }
       serde_json::from_slice(&output.stdout).map_err(CniError::ConfigParse)
   }
   ```
   Note: the JSON fixture strings in the test use escaped quotes (`\"`) because they're embedded
   in a shell-script `echo` string built with Rust's `format!` — this matches exactly what a real
   CNI plugin binary does (emit one JSON line on stdout).

3. Verify:
   ```
   cargo nextest run -p minibox-cni    → all green
   cargo clippy -p minibox-cni -- -D warnings    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox-cni): add exec_plugin happy-path + not-found handling"`

---

### Task 6: `exec_plugin` — structured CNI error payload

**Crate**: `minibox-cni`
**File(s)**: `crates/minibox-cni/src/exec.rs`
**Run**: `cargo nextest run -p minibox-cni`

1. Write failing test (append inside the existing `#[cfg(test)] mod tests` block in
   `crates/minibox-cni/src/exec.rs`):
   ```rust
   #[tokio::test]
   async fn exec_plugin_structured_error_returns_plugin_error() {
       let dir = tempfile::tempdir().expect("tempdir");
       let path = dir.path().join("fake-host-local");
       let script = "#!/bin/sh\necho '{\"code\":7,\"msg\":\"no IPs available\",\"details\":\"pool exhausted\"}'\nexit 1\n";
       std::fs::write(&path, script).expect("write fixture plugin");
       let mut perms = std::fs::metadata(&path).expect("stat").permissions();
       perms.set_mode(0o755);
       std::fs::set_permissions(&path, perms).expect("chmod");

       let plugin = PluginConfig {
           plugin_type: "fake-host-local".to_string(),
           raw: json!({"type": "fake-host-local"}),
       };

       let result = exec_plugin(
           &[dir.path().to_path_buf()],
           &plugin,
           "ADD",
           "/fake/netns",
           "container-1",
           "eth0",
           None,
       )
       .await;

       match result {
           Err(CniError::PluginError { plugin, code, msg, details }) => {
               assert_eq!(plugin, "fake-host-local");
               assert_eq!(code, Some(7));
               assert_eq!(msg, "no IPs available");
               assert_eq!(details.as_deref(), Some("pool exhausted"));
           }
           other => panic!("expected PluginError, got {other:?}"),
       }
   }
   ```
   Run: `cargo nextest run -p minibox-cni -- exec_plugin_structured_error_returns_plugin_error`
   Expected: FAIL — the fixture binary name collides with nothing yet defined, so this actually
   compiles against Task 5's `exec_plugin` already; run it to confirm it currently **passes**
   only if Task 5's error-handling branch already covers this shape. If it fails, the gap is that
   `exec_plugin` doesn't yet distinguish structured-error stdout from plain non-zero exit — Task
   5's implementation above already includes this branch, so this test should pass immediately.
   Treat this task as **characterization**: it locks in behavior Task 5 already implements,
   giving it dedicated coverage rather than relying on Task 5's tests to exercise it incidentally.

2. No implementation changes needed — Task 5's `exec_plugin` already parses
   `CniErrorPayload` from stdout on non-zero exit. This task exists to give that branch its own
   explicit, named test rather than leaving it implicitly covered.

3. Verify:
   ```
   cargo nextest run -p minibox-cni    → all green
   cargo clippy -p minibox-cni -- -D warnings    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "test(minibox-cni): cover exec_plugin structured CNI error path"`

---

### Task 7: `exec_plugin` — process failure without structured error

**Crate**: `minibox-cni`
**File(s)**: `crates/minibox-cni/src/exec.rs`
**Run**: `cargo nextest run -p minibox-cni`

1. Write failing test (append inside `#[cfg(test)] mod tests` in
   `crates/minibox-cni/src/exec.rs`):
   ```rust
   #[tokio::test]
   async fn exec_plugin_crash_without_structured_error_returns_process_failed() {
       let dir = tempfile::tempdir().expect("tempdir");
       let path = dir.path().join("fake-crash");
       let script = "#!/bin/sh\necho 'not json, just a crash log line' >&2\nexit 2\n";
       std::fs::write(&path, script).expect("write fixture plugin");
       let mut perms = std::fs::metadata(&path).expect("stat").permissions();
       perms.set_mode(0o755);
       std::fs::set_permissions(&path, perms).expect("chmod");

       let plugin = PluginConfig {
           plugin_type: "fake-crash".to_string(),
           raw: json!({"type": "fake-crash"}),
       };

       let result = exec_plugin(
           &[dir.path().to_path_buf()],
           &plugin,
           "ADD",
           "/fake/netns",
           "container-1",
           "eth0",
           None,
       )
       .await;

       match result {
           Err(CniError::ProcessFailed { plugin, exit_code, stderr }) => {
               assert_eq!(plugin, "fake-crash");
               assert_eq!(exit_code, Some(2));
               assert!(stderr.contains("crash log line"));
           }
           other => panic!("expected ProcessFailed, got {other:?}"),
       }
   }
   ```
   Run: `cargo nextest run -p minibox-cni -- exec_plugin_crash_without_structured_error_returns_process_failed`
   Expected: this exercises the fallback branch already present in Task 5's implementation
   (non-JSON stdout on non-zero exit → `ProcessFailed`). Same characterization note as Task 6.

2. No implementation changes needed — same rationale as Task 6.

3. Verify:
   ```
   cargo nextest run -p minibox-cni    → all green
   cargo clippy -p minibox-cni -- -D warnings    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "test(minibox-cni): cover exec_plugin raw process-failure path"`

---

### Task 8: `NetworkConfigList::add` — chain orchestration happy path

**Crate**: `minibox-cni`
**File(s)**: `crates/minibox-cni/src/config.rs`
**Run**: `cargo nextest run -p minibox-cni`

1. Write failing test (append inside `#[cfg(test)] mod tests` in
   `crates/minibox-cni/src/config.rs`):
   ```rust
   #[allow(clippy::unwrap_used)]
   fn write_executable(dir: &std::path::Path, name: &str, script: &str) {
       use std::os::unix::fs::PermissionsExt;
       let path = dir.join(name);
       std::fs::write(&path, script).unwrap();
       let mut perms = std::fs::metadata(&path).unwrap().permissions();
       perms.set_mode(0o755);
       std::fs::set_permissions(&path, perms).unwrap();
   }

   #[tokio::test]
   async fn add_threads_prev_result_across_chain() {
       let bin_dir = tempfile::tempdir().expect("bin tempdir");
       // First plugin ignores prevResult (there is none), emits an IP.
       write_executable(
           bin_dir.path(),
           "fake-bridge",
           "#!/bin/sh\necho '{\"cniVersion\":\"1.0.0\",\"ips\":[{\"address\":\"10.0.0.5/24\"}]}'\nexit 0\n",
       );
       // Second plugin reads stdin, asserts prevResult's IP made it through, then re-emits it.
       write_executable(
           bin_dir.path(),
           "fake-portmap",
           "#!/bin/sh\ncat > /tmp/minibox-cni-test-portmap-input.json\n\
            if grep -q '10.0.0.5/24' /tmp/minibox-cni-test-portmap-input.json; then\n\
            echo '{\"cniVersion\":\"1.0.0\",\"ips\":[{\"address\":\"10.0.0.5/24\"}]}'\nexit 0\n\
            else\nexit 1\nfi\n",
       );

       let list = NetworkConfigList {
           cni_version: "1.0.0".to_string(),
           name: "minibox0".to_string(),
           plugins: vec![
               PluginConfig { plugin_type: "fake-bridge".to_string(), raw: serde_json::json!({"type": "fake-bridge"}) },
               PluginConfig { plugin_type: "fake-portmap".to_string(), raw: serde_json::json!({"type": "fake-portmap"}) },
           ],
       };

       let result = list
           .add(&[bin_dir.path().to_path_buf()], "/fake/netns", "container-1", "eth0")
           .await
           .expect("add should succeed");

       assert_eq!(result.ips[0].address, "10.0.0.5/24");
       let _ = std::fs::remove_file("/tmp/minibox-cni-test-portmap-input.json");
   }
   ```
   Run: `cargo nextest run -p minibox-cni -- add_threads_prev_result_across_chain`
   Expected: FAIL (`NetworkConfigList::add` does not exist yet)

2. Implement (append to `crates/minibox-cni/src/config.rs`, after the existing `impl
   NetworkConfigList` block's `from_file` method, inside the same `impl` block):
   ```rust
       /// Run the full ADD chain in plugin order, threading `prevResult`
       /// between plugins. On mid-chain failure, rolls back (`DEL` in
       /// reverse) the already-succeeded plugins before returning the error.
       ///
       /// # Errors
       ///
       /// Returns the first plugin failure encountered, after best-effort
       /// rollback of any plugins that had already succeeded.
       #[tracing::instrument(skip(self), fields(network = %self.name, plugin_count = self.plugins.len()))]
       pub async fn add(
           &self,
           cni_path: &[std::path::PathBuf],
           netns: &str,
           container_id: &str,
           ifname: &str,
       ) -> Result<crate::result::CniResult, CniError> {
           let mut prev_result: Option<serde_json::Value> = None;
           let mut succeeded: Vec<&PluginConfig> = Vec::new();

           for plugin in &self.plugins {
               match crate::exec::exec_plugin(
                   cni_path,
                   plugin,
                   "ADD",
                   netns,
                   container_id,
                   ifname,
                   prev_result.as_ref(),
               )
               .await
               {
                   Ok(result) => {
                       succeeded.push(plugin);
                       prev_result = Some(result);
                   }
                   Err(err) => {
                       for rollback_plugin in succeeded.iter().rev() {
                           if let Err(rollback_err) = crate::exec::exec_plugin(
                               cni_path,
                               rollback_plugin,
                               "DEL",
                               netns,
                               container_id,
                               ifname,
                               None,
                           )
                           .await
                           {
                               tracing::warn!(
                                   plugin = %rollback_plugin.plugin_type,
                                   error = %rollback_err,
                                   "cni: rollback DEL failed after mid-chain ADD failure"
                               );
                           }
                       }
                       return Err(err);
                   }
               }
           }

           let final_result = prev_result.ok_or_else(|| CniError::PluginError {
               plugin: self.name.clone(),
               code: None,
               msg: "empty plugin chain produced no result".to_string(),
               details: None,
           })?;
           serde_json::from_value(final_result).map_err(CniError::ConfigParse)
       }
   ```

3. Verify:
   ```
   cargo nextest run -p minibox-cni    → all green
   cargo clippy -p minibox-cni -- -D warnings    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox-cni): add NetworkConfigList::add chain orchestration"`

---

### Task 9: `NetworkConfigList::add` — mid-chain failure rollback

**Crate**: `minibox-cni`
**File(s)**: `crates/minibox-cni/src/config.rs`
**Run**: `cargo nextest run -p minibox-cni`

1. Write failing test (append inside `#[cfg(test)] mod tests` in
   `crates/minibox-cni/src/config.rs`):
   ```rust
   #[tokio::test]
   async fn add_rolls_back_succeeded_plugins_on_mid_chain_failure() {
       let bin_dir = tempfile::tempdir().expect("bin tempdir");
       let del_marker = bin_dir.path().join("bridge-was-deleted");

       // Plugin 1 succeeds on ADD, and on DEL touches a marker file so the
       // test can assert rollback actually ran.
       write_executable(
           bin_dir.path(),
           "fake-bridge",
           &format!(
               "#!/bin/sh\nif [ \"$CNI_COMMAND\" = \"ADD\" ]; then\n  echo '{{\"cniVersion\":\"1.0.0\"}}'\nelif [ \"$CNI_COMMAND\" = \"DEL\" ]; then\n  touch {}\nfi\nexit 0\n",
               del_marker.display()
           ),
       );
       // Plugin 2 always fails ADD with a structured error.
       write_executable(
           bin_dir.path(),
           "fake-portmap",
           "#!/bin/sh\necho '{\"code\":9,\"msg\":\"boom\"}'\nexit 1\n",
       );

       let list = NetworkConfigList {
           cni_version: "1.0.0".to_string(),
           name: "minibox0".to_string(),
           plugins: vec![
               PluginConfig { plugin_type: "fake-bridge".to_string(), raw: serde_json::json!({"type": "fake-bridge"}) },
               PluginConfig { plugin_type: "fake-portmap".to_string(), raw: serde_json::json!({"type": "fake-portmap"}) },
           ],
       };

       let result = list
           .add(&[bin_dir.path().to_path_buf()], "/fake/netns", "container-1", "eth0")
           .await;

       assert!(matches!(result, Err(CniError::PluginError { code: Some(9), .. })));
       assert!(del_marker.exists(), "rollback DEL should have run for the succeeded plugin");
   }
   ```
   Run: `cargo nextest run -p minibox-cni -- add_rolls_back_succeeded_plugins_on_mid_chain_failure`
   Expected: this exercises the rollback branch already implemented in Task 8. Same
   characterization rationale as Tasks 6/7 — Task 8's `add` already performs reverse-order DEL on
   failure; this test gives that specific behavior its own dedicated, named coverage.

2. No implementation changes needed.

3. Verify:
   ```
   cargo nextest run -p minibox-cni    → all green
   cargo clippy -p minibox-cni -- -D warnings    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "test(minibox-cni): cover add() mid-chain rollback"`

---

### Task 10: `NetworkConfigList::del` — reverse-order best-effort teardown

**Crate**: `minibox-cni`
**File(s)**: `crates/minibox-cni/src/config.rs`
**Run**: `cargo nextest run -p minibox-cni`

1. Write failing test (append inside `#[cfg(test)] mod tests` in
   `crates/minibox-cni/src/config.rs`):
   ```rust
   #[tokio::test]
   async fn del_runs_all_plugins_in_reverse_even_if_one_fails() {
       let bin_dir = tempfile::tempdir().expect("bin tempdir");
       let second_marker = bin_dir.path().join("second-plugin-deleted");

       // First in chain order (last in DEL order) always fails.
       write_executable(bin_dir.path(), "fake-bridge", "#!/bin/sh\nexit 1\n");
       // Second in chain order (first in DEL order) succeeds and leaves a marker.
       write_executable(
           bin_dir.path(),
           "fake-portmap",
           &format!("#!/bin/sh\ntouch {}\nexit 0\n", second_marker.display()),
       );

       let list = NetworkConfigList {
           cni_version: "1.0.0".to_string(),
           name: "minibox0".to_string(),
           plugins: vec![
               PluginConfig { plugin_type: "fake-bridge".to_string(), raw: serde_json::json!({"type": "fake-bridge"}) },
               PluginConfig { plugin_type: "fake-portmap".to_string(), raw: serde_json::json!({"type": "fake-portmap"}) },
           ],
       };

       let result = list
           .del(&[bin_dir.path().to_path_buf()], "/fake/netns", "container-1", "eth0")
           .await;

       assert!(result.is_ok(), "del() should return Ok even if a plugin DEL fails");
       assert!(second_marker.exists(), "the succeeding plugin's DEL should still have run");
   }
   ```
   Run: `cargo nextest run -p minibox-cni -- del_runs_all_plugins_in_reverse_even_if_one_fails`
   Expected: FAIL (`NetworkConfigList::del` does not exist yet)

2. Implement (append to `crates/minibox-cni/src/config.rs`, inside the same `impl
   NetworkConfigList` block, after `add`):
   ```rust
       /// Run the DEL chain in reverse plugin order. Individual plugin DEL
       /// failures are logged and do not short-circuit remaining teardown
       /// steps — matches the CNI spec's expectation that DEL is idempotent
       /// and best-effort.
       #[tracing::instrument(skip(self), fields(network = %self.name, plugin_count = self.plugins.len()))]
       pub async fn del(
           &self,
           cni_path: &[std::path::PathBuf],
           netns: &str,
           container_id: &str,
           ifname: &str,
       ) -> Result<(), CniError> {
           for plugin in self.plugins.iter().rev() {
               if let Err(err) = crate::exec::exec_plugin(
                   cni_path,
                   plugin,
                   "DEL",
                   netns,
                   container_id,
                   ifname,
                   None,
               )
               .await
               {
                   tracing::warn!(
                       plugin = %plugin.plugin_type,
                       error = %err,
                       "cni: plugin DEL failed during teardown"
                   );
               }
           }
           Ok(())
       }
   ```

3. Verify:
   ```
   cargo nextest run -p minibox-cni    → all green
   cargo clippy -p minibox-cni -- -D warnings    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox-cni): add NetworkConfigList::del best-effort teardown"`

---

### Task 11: `CniNetworkProvider` — implement `NetworkProvider`

**Crate**: `minibox-cni`
**File(s)**: `crates/minibox-cni/src/provider.rs`
**Run**: `cargo nextest run -p minibox-cni`

1. Write failing test (append to `crates/minibox-cni/src/provider.rs`):
   ```rust
   #[cfg(test)]
   #[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
   mod tests {
       use super::*;
       use minibox_core::domain::NetworkMode;
       use std::io::Write;
       use std::os::unix::fs::PermissionsExt;

       fn write_conflist(dir: &std::path::Path) {
           let mut file = std::fs::File::create(dir.join("10-minibox.conflist")).expect("create conflist");
           file.write_all(
               br#"{"cniVersion":"1.0.0","name":"minibox0","plugins":[{"type":"fake-noop"}]}"#,
           )
           .expect("write conflist");
       }

       fn write_noop_plugin(dir: &std::path::Path) {
           let path = dir.join("fake-noop");
           std::fs::write(&path, "#!/bin/sh\nif [ \"$CNI_COMMAND\" = \"ADD\" ]; then\n  echo '{\"cniVersion\":\"1.0.0\"}'\nfi\nexit 0\n").expect("write plugin");
           let mut perms = std::fs::metadata(&path).unwrap().permissions();
           perms.set_mode(0o755);
           std::fs::set_permissions(&path, perms).unwrap();
       }

       #[tokio::test]
       async fn attach_then_cleanup_uses_recorded_netns() {
           let config_dir = tempfile::tempdir().expect("config dir");
           let bin_dir = tempfile::tempdir().expect("bin dir");
           write_conflist(config_dir.path());
           write_noop_plugin(bin_dir.path());

           let provider = CniNetworkProvider::new(
               vec![bin_dir.path().to_path_buf()],
               config_dir.path().to_path_buf(),
           );

           let config = minibox_core::domain::NetworkConfig {
               mode: NetworkMode::Bridge,
               ..Default::default()
           };
           provider.setup("container-1", &config).await.expect("setup should succeed");
           provider.attach("container-1", 1).await.expect("attach should succeed");
           provider.cleanup("container-1").await.expect("cleanup should succeed");
       }

       #[tokio::test]
       async fn cleanup_without_prior_attach_returns_error() {
           let config_dir = tempfile::tempdir().expect("config dir");
           write_conflist(config_dir.path());
           let provider = CniNetworkProvider::new(vec![], config_dir.path().to_path_buf());

           let result = provider.cleanup("never-attached").await;
           assert!(result.is_err());
       }
   }
   ```
   Run: `cargo nextest run -p minibox-cni -- attach_then_cleanup_uses_recorded_netns`
   Expected: FAIL (`CniNetworkProvider` does not exist yet)

2. Implement (prepend to `crates/minibox-cni/src/provider.rs`, above the `#[cfg(test)]` block):
   ```rust
   //! `NetworkProvider` adapter implementing minibox-core's port via CNI
   //! plugin chains.

   use crate::config::NetworkConfigList;
   use anyhow::Context as _;
   use async_trait::async_trait;
   use minibox_core::domain::{AsAny, NetworkConfig, NetworkProvider, NetworkStats};
   use std::any::Any;
   use std::path::PathBuf;
   use tracing::instrument;

   /// Adapter implementing minibox-core's `NetworkProvider` port via CNI
   /// plugin chains. Tracks the network namespace path used for each
   /// container's `attach()` call so `cleanup()` (which only receives a
   /// `container_id`) can run the matching `DEL` chain.
   #[derive(Debug)]
   pub struct CniNetworkProvider {
       /// Directories searched for plugin binaries, in order.
       pub cni_path: Vec<PathBuf>,
       /// Directory containing the `.conflist` network configuration.
       pub config_dir: PathBuf,
       netns_by_container: dashmap::DashMap<String, String>,
   }

   impl CniNetworkProvider {
       /// Create a new CNI-backed network provider.
       #[must_use]
       pub fn new(cni_path: Vec<PathBuf>, config_dir: PathBuf) -> Self {
           Self {
               cni_path,
               config_dir,
               netns_by_container: dashmap::DashMap::new(),
           }
       }

       fn conflist_path(&self) -> PathBuf {
           self.config_dir.join("10-minibox.conflist")
       }
   }

   impl AsAny for CniNetworkProvider {
       fn as_any(&self) -> &dyn Any {
           self
       }
   }

   #[async_trait]
   impl NetworkProvider for CniNetworkProvider {
       #[instrument(skip(self, _config), fields(container_id = %container_id))]
       async fn setup(&self, container_id: &str, _config: &NetworkConfig) -> anyhow::Result<String> {
           // Namespace attach happens in `attach()` once the container PID is
           // known; `setup()` only validates the .conflist is present and
           // parseable up front, so misconfiguration fails fast before the
           // container process is even spawned.
           NetworkConfigList::from_file(&self.conflist_path()).context("loading CNI .conflist")?;
           Ok(container_id.to_string())
       }

       #[instrument(skip(self), fields(container_id = %container_id, pid = pid))]
       async fn attach(&self, container_id: &str, pid: u32) -> anyhow::Result<()> {
           let conflist =
               NetworkConfigList::from_file(&self.conflist_path()).context("loading CNI .conflist")?;
           let netns = format!("/proc/{pid}/ns/net");
           conflist
               .add(&self.cni_path, &netns, container_id, "eth0")
               .await
               .context("running CNI ADD chain")?;
           self.netns_by_container
               .insert(container_id.to_string(), netns);
           Ok(())
       }

       #[instrument(skip(self), fields(container_id = %container_id))]
       async fn cleanup(&self, container_id: &str) -> anyhow::Result<()> {
           let (_, netns) = self
               .netns_by_container
               .remove(container_id)
               .ok_or_else(|| anyhow::anyhow!("no recorded network namespace for container {container_id}"))?;
           let conflist =
               NetworkConfigList::from_file(&self.conflist_path()).context("loading CNI .conflist")?;
           conflist
               .del(&self.cni_path, &netns, container_id, "eth0")
               .await
               .context("running CNI DEL chain")?;
           Ok(())
       }

       async fn stats(&self, _container_id: &str) -> anyhow::Result<NetworkStats> {
           anyhow::bail!("CniNetworkProvider does not yet implement stats collection")
       }
   }
   ```

3. Verify:
   ```
   cargo nextest run -p minibox-cni    → all green
   cargo clippy -p minibox-cni -- -D warnings    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox-cni): implement NetworkProvider via CniNetworkProvider"`

---

### Task 12: Wire `cni` feature flag

**Crate**: `minibox`, `miniboxd`
**File(s)**: `crates/minibox/Cargo.toml`, `crates/miniboxd/Cargo.toml`
**Run**: `cargo check -p miniboxd --features cni` and `cargo check -p miniboxd`

1. No new test — this is Cargo manifest wiring. Verification is compiling both with and without
   the feature (Task 13 adds the code path this feature gates, so this task alone doesn't change
   any runtime behavior yet; it exists as its own commit so Task 13's diff is pure logic).

2. Edit `crates/minibox/Cargo.toml` `[dependencies]` section — add (alphabetical position, near
   other `minibox-*` deps):
   ```toml
   minibox-cni = { workspace = true, optional = true }
   ```
   Edit `crates/minibox/Cargo.toml` `[features]` section — add after the existing `otel` block:
   ```toml
   ## CNI (Container Network Interface) plugin-based bridge networking for the
   ## native Linux adapter. Requires CNI plugin binaries (bridge, host-local,
   ## portmap, dnsname) installed and MINIBOX_CNI_PATH set — see
   ## docs/PLATFORM_SUPPORT.md. Off by default: with it disabled, the native
   ## suite keeps constructing the existing bespoke BridgeNetwork.
   cni = ["dep:minibox-cni"]
   ```

3. Edit `crates/miniboxd/Cargo.toml` `[features]` section — add:
   ```toml
   cni = ["minibox/cni"]
   ```
   (Do not add `cni` to `default = ["metrics", "otel"]` — stays opt-in per the design's rollout
   decision.)

4. Verify:
   ```
   cargo check -p miniboxd --features cni    → compiles (minibox-cni now an optional dep)
   cargo check -p miniboxd                   → compiles (feature off, unchanged behavior)
   ```

5. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox): add opt-in cni feature flag"`

---

### Task 13: Wire `CniNetworkProvider` into `resolve_native_network()`

**Crate**: `miniboxd`
**File(s)**: `crates/miniboxd/src/main.rs`
**Run**: `cargo check -p miniboxd --features cni` and `cargo check -p miniboxd`

1. No new automated test — this function selects a `NetworkProvider` implementation based on
   `MINIBOX_NETWORK_MODE` and a compile-time feature; it's exercised indirectly by the daemon's
   existing e2e/integration suites once merged, and by manual verification (Task 17 covers the
   `mbx doctor` preflight check that surfaces whether the feature is actually usable). Verify via
   compilation with each feature combination, matching this function's existing pattern of
   `#[cfg(feature = "tailnet")]` gating one match arm.

2. Locate the `"bridge"` arm inside `resolve_native_network()` (`crates/miniboxd/src/main.rs`,
   inside the existing `match mode.as_str() { ... }` block) and replace:
   ```rust
           "bridge" => Ok(Arc::new(
               BridgeNetwork::new().context("BridgeNetwork init failed")?,
           )),
   ```
   with:
   ```rust
           #[cfg(feature = "cni")]
           "bridge" => {
               let cni_path = std::env::var("MINIBOX_CNI_PATH")
                   .unwrap_or_else(|_| "/opt/cni/bin".to_string());
               let cni_path: Vec<std::path::PathBuf> =
                   std::env::split_paths(&cni_path).collect();
               let config_dir = std::env::var("MINIBOX_CNI_CONFIG_DIR")
                   .unwrap_or_else(|_| "/etc/cni/net.d".to_string());
               Ok(Arc::new(minibox_cni::CniNetworkProvider::new(
                   cni_path,
                   std::path::PathBuf::from(config_dir),
               )))
           }
           #[cfg(not(feature = "cni"))]
           "bridge" => Ok(Arc::new(
               BridgeNetwork::new().context("BridgeNetwork init failed")?,
           )),
   ```
   This preserves the design's decision exactly: same `"bridge"` mode string, cfg-gated
   construction — no runtime toggle, no dual construction, `BridgeNetwork` stays the default with
   the feature off.

3. Verify:
   ```
   cargo check -p miniboxd --features cni    → compiles, uses CniNetworkProvider for "bridge"
   cargo check -p miniboxd                   → compiles, still uses BridgeNetwork for "bridge"
   cargo clippy -p miniboxd --features cni -- -D warnings    → zero warnings
   cargo clippy -p miniboxd -- -D warnings                   → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(miniboxd): construct CniNetworkProvider for bridge mode behind cni feature"`

---

### Task 14: `#[instrument]` on `handle_run` — daemon request boundary span

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/daemon/handler/run.rs`
**Run**: `cargo nextest run -p minibox -- handle_run`

1. No new test — instrumentation is additive to an existing, already-tested function; behavior
   must not change. Verify via the existing test suite for `run.rs` staying green (run it before
   and after to confirm no regression):
   ```
   cargo nextest run -p minibox -- daemon::handler::run::
   ```
   Expected before change: existing tests pass (establishes baseline).

2. Edit `crates/minibox/src/daemon/handler/run.rs` — add the instrument attribute immediately
   above the `pub async fn handle_run(` signature (matching the exact style already used in
   `crates/minibox/src/daemon/handler/image.rs:77`):
   ```rust
   #[instrument(skip(state, deps, tx), fields(image = %params.image, ephemeral = params.ephemeral))]
   pub async fn handle_run(
       params: RunParams,
       state: Arc<DaemonState>,
       deps: Arc<HandlerDependencies>,
       tx: mpsc::Sender<DaemonResponse>,
   ) {
   ```
   Confirm `use tracing::instrument;` is present in this file's imports (add it if missing,
   alongside the file's existing `tracing::{...}` import if one exists — check the top of
   `run.rs` for the current import list before adding a duplicate).

3. Verify:
   ```
   cargo nextest run -p minibox -- daemon::handler::run::    → all green, unchanged
   cargo clippy -p minibox -- -D warnings                    → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox): instrument handle_run with OTEL span"`

---

### Task 15: `#[instrument]` on `LinuxNamespaceRuntime::spawn_process` — adapter boundary span

**Crate**: `minibox`
**File(s)**: `crates/minibox/src/adapters/runtime.rs`
**Run**: `cargo nextest run -p minibox -- runtime::`

1. No new test — same rationale as Task 14. Verify existing `runtime.rs` tests stay green before
   and after.

2. Edit `crates/minibox/src/adapters/runtime.rs` — add the instrument attribute immediately above
   the `spawn_process` method inside `impl ContainerRuntime for LinuxNamespaceRuntime` (line
   109):
   ```rust
   #[instrument(skip(self, config), fields(command = %config.command, privileged = config.privileged), err)]
   async fn spawn_process(&self, config: &ContainerSpawnConfig) -> anyhow::Result<SpawnResult> {
   ```
   Confirm `use tracing::instrument;` is present in this file's imports (add if missing).

3. Verify:
   ```
   cargo nextest run -p minibox -- runtime::    → all green, unchanged
   cargo clippy -p minibox -- -D warnings       → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(minibox): instrument LinuxNamespaceRuntime::spawn_process with OTEL span"`

---

### Task 16: `mbx doctor` — CNI plugin/`CNI_PATH` preflight check

**Crate**: `mbx`
**File(s)**: `crates/mbx/src/commands/doctor.rs`
**Run**: `cargo nextest run -p mbx -- doctor::`

1. Write failing test (append a `#[cfg(test)] mod tests` block, or extend the existing one if
   `doctor.rs` already has one — check the file first and follow whichever is the case):
   ```rust
   #[test]
   fn cni_plugin_status_reports_missing_binaries_when_path_unset() {
       // SAFETY: test runs single-threaded per this module's env-var guard convention;
       // MINIBOX_CNI_PATH is unset in the test process by default.
       let statuses = cni_plugin_status();
       assert!(statuses.iter().any(|s| !s.found));
   }

   #[test]
   fn cni_plugin_status_reports_found_when_binaries_present() {
       let dir = tempfile::tempdir().expect("tempdir");
       for name in ["bridge", "host-local", "portmap", "dnsname"] {
           std::fs::write(dir.path().join(name), "#!/bin/sh\n").expect("write fixture");
       }
       // SAFETY: guarded by the module's shared env-mutation lock (see MUTEX below).
       let _guard = ENV_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
       unsafe { std::env::set_var("MINIBOX_CNI_PATH", dir.path()) };
       let statuses = cni_plugin_status();
       unsafe { std::env::remove_var("MINIBOX_CNI_PATH") };
       assert!(statuses.iter().all(|s| s.found));
   }
   ```
   Run: `cargo nextest run -p mbx -- cni_plugin_status_reports_missing_binaries_when_path_unset`
   Expected: FAIL (`cni_plugin_status` does not exist yet)

2. Implement — add near the existing `compiled_adapters()`/`selected_adapter()` functions in
   `crates/mbx/src/commands/doctor.rs`:
   ```rust
   /// Result of checking a single CNI plugin binary's presence.
   pub struct CniPluginStatus {
       pub plugin: &'static str,
       pub found: bool,
   }

   /// Check whether the standard CNI plugin binaries minibox's native adapter
   /// needs (when built with the `cni` feature) are present on
   /// `MINIBOX_CNI_PATH` (defaulting to `/opt/cni/bin`). Advisory only — does
   /// not invoke the binaries, just checks presence, matching this module's
   /// existing checks' style.
   pub fn cni_plugin_status() -> Vec<CniPluginStatus> {
       let cni_path = std::env::var("MINIBOX_CNI_PATH").unwrap_or_else(|_| "/opt/cni/bin".to_string());
       let dirs: Vec<std::path::PathBuf> = std::env::split_paths(&cni_path).collect();
       ["bridge", "host-local", "portmap", "dnsname"]
           .into_iter()
           .map(|plugin| CniPluginStatus {
               plugin,
               found: dirs.iter().any(|dir| dir.join(plugin).is_file()),
           })
           .collect()
   }
   ```
   Also add a module-level `static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());`
   near the top of the test module if this module doesn't already have one for env-mutating tests
   (per this repo's env-mutation convention) — check the file first; if a shared lock already
   exists for other tests in this file, reuse it instead of declaring a second one.

   Then wire the check into `execute()` alongside the existing adapter checks — add a call to
   `cni_plugin_status()` and print each plugin's found/missing status in the same style as the
   existing `compiled_adapters()` output block (match the exact print formatting already used for
   that list — read `execute()`'s current body before inserting to match indentation/format
   exactly).

3. Verify:
   ```
   cargo nextest run -p mbx -- doctor::    → all green
   cargo clippy -p mbx -- -D warnings      → zero warnings
   ```

4. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "feat(mbx): add CNI plugin preflight check to mbx doctor"`

---

### Task 17: Justfile + docs update

**Crate**: none (repo-level)
**File(s)**: `justfile`, `docs/PLATFORM_SUPPORT.md`
**Run**: `cargo nextest run -p minibox-cni -p minibox -p minibox-core -p minibox-macros -p minibox-crux-plugin -p mbx -p miniboxd`

1. No test — documentation and CI-recipe update. Verification is the full nextest recipe run
   listed above passing with `minibox-cni` included.

2. Edit `justfile`'s `nextest` recipe (confirmed at lines 82-84) to add `-p minibox-cni`:
   ```
   nextest:
       cargo nextest run --release -p minibox -p minibox-core -p minibox-macros -p minibox-crux-plugin -p mbx -p miniboxd -p minibox-cni
   ```

3. Edit `docs/PLATFORM_SUPPORT.md`'s `### smolvm (default)` / native adapter prerequisites
   section — add a new subsection documenting the opt-in CNI requirement (read the file's current
   structure first to match its existing heading/bullet style exactly; add prose covering):
   - `cni` feature (off by default) requires the standard `containernetworking/plugins` release
     binaries (`bridge`, `host-local`, `portmap`, `dnsname`) on `MINIBOX_CNI_PATH`
     (default `/opt/cni/bin`)
   - A `.conflist` at `MINIBOX_CNI_CONFIG_DIR` (default `/etc/cni/net.d/10-minibox.conflist`)
   - With the feature off, native bridge networking is unchanged (existing `BridgeNetwork`)
   - `mbx doctor` reports plugin presence

4. Verify:
   ```
   cargo nextest run --release -p minibox -p minibox-core -p minibox-macros -p minibox-crux-plugin -p mbx -p miniboxd -p minibox-cni    → all green
   ```

5. Run: `git branch --show-current`
   Verify output is `develop`. Stop immediately if not.
   Commit: `git commit -m "docs: document opt-in CNI networking prerequisites"`
