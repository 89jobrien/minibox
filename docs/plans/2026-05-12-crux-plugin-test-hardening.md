# Plan: crux-plugin integration test hardening

## Goal

Eliminate flaky timing, reduce boilerplate, and close coverage gaps in
`minibox-crux-plugin` integration tests (#336-#340).

## Architecture

- Crate affected: `minibox-crux-plugin` (tests only)
- No new production types or traits
- New test infrastructure: `PluginHarness` struct, `mock_daemon_verify` helper
- Data flow: test -> PluginHarness (stdin/stdout) -> plugin binary ->
  UnixListener mock -> DaemonRequest captured via oneshot channel

## Tech Stack

- Rust 2024, tokio, serde_json, tempfile
- `tokio::sync::oneshot` for request capture (already a transitive dep)
- No new dependencies

## Tasks

### Task 1: Extract PluginHarness struct (#336)

**Crate**: `minibox-crux-plugin`
**File(s)**: `crates/minibox-crux-plugin/tests/integration.rs`
**Run**: `cargo test -p minibox-crux-plugin`

1. Add `PluginHarness` struct at the top of the helpers section:

   ```rust
   struct PluginHarness {
       stdin: tokio::process::ChildStdin,
       stdout: BufReader<tokio::process::ChildStdout>,
       child: tokio::process::Child,
   }

   impl PluginHarness {
       fn spawn(socket_path: &Path) -> Self {
           let mut child = spawn_plugin(socket_path);
           let stdin = child.stdin.take().expect("piped stdin");
           let stdout = child.stdout.take().expect("piped stdout");
           Self {
               stdin,
               stdout: BufReader::new(stdout),
               child,
           }
       }

       async fn send(&mut self, value: &Value) {
           send(&mut self.stdin, value).await;
       }

       async fn recv(&mut self) -> Value {
           recv(&mut self.stdout).await
       }

       async fn invoke(&mut self, handler: &str, input: Value) -> Value {
           self.send(&json!({
               "method": "Invoke",
               "params": { "handler": handler, "input": input }
           })).await;
           self.recv().await
       }

       async fn shutdown(mut self) -> std::process::ExitStatus {
           self.send(&json!({"method": "Shutdown"})).await;
           let ack = self.recv().await;
           assert_eq!(ack["status"], "ShutdownAck");
           self.child.wait().await.expect("child wait")
       }
   }
   ```

2. Rewrite all existing tests to use `PluginHarness`. Example for
   `declare_returns_nine_handlers`:

   ```rust
   #[tokio::test]
   async fn declare_returns_thirteen_handlers() {
       let tmp = TempDir::new().expect("tempdir");
       let socket_path = tmp.path().join("daemon.sock");
       let mut h = PluginHarness::spawn(&socket_path);

       h.send(&json!({"method": "Declare"})).await;
       let resp = h.recv().await;
       assert_eq!(resp["status"], "Declare");
       let handlers = resp["data"]["handlers"]
           .as_array()
           .expect("handlers array");
       assert_eq!(handlers.len(), 13);

       h.shutdown().await;
   }
   ```

3. Verify:
   ```
   cargo test -p minibox-crux-plugin    -> all 41 tests pass
   cargo clippy -p minibox-crux-plugin -- -D warnings  -> zero warnings
   ```

4. Commit: `refactor(crux-plugin): extract PluginHarness to reduce test boilerplate`

### Task 2: Replace sleep with bind-before-spawn (#337)

**Crate**: `minibox-crux-plugin`
**File(s)**: `crates/minibox-crux-plugin/tests/integration.rs`
**Run**: `cargo test -p minibox-crux-plugin`

1. Change `mock_daemon_once` to accept a pre-bound `UnixListener` instead of
   a `PathBuf`:

   ```rust
   async fn mock_daemon_once(listener: UnixListener, response: DaemonResponse) {
       let (stream, _) = listener.accept().await.expect("accept mock connection");
       let (read_half, mut write_half) = tokio::io::split(stream);
       let mut reader = BufReader::new(read_half);
       let mut line = String::new();
       reader.read_line(&mut line).await.expect("read request line");
       let mut resp = serde_json::to_string(&response).expect("serialize");
       resp.push('\n');
       write_half.write_all(resp.as_bytes()).await.expect("write");
       write_half.flush().await.expect("flush");
   }
   ```

2. Do the same for `mock_daemon_multi`.

3. At each call site, bind the listener before spawning the plugin:

   ```rust
   let listener = UnixListener::bind(&socket_path).expect("bind");
   tokio::spawn(mock_daemon_once(listener, response));
   // No sleep -- socket already exists
   let mut h = PluginHarness::spawn(&socket_path);
   ```

4. Remove all `tokio::time::sleep` calls and the
   `use std::time::Duration` import if now unused.

5. Verify:
   ```
   cargo test -p minibox-crux-plugin    -> all pass
   cargo clippy -p minibox-crux-plugin -- -D warnings  -> zero warnings
   ```

6. Commit: `fix(crux-plugin): replace sleep(10ms) with bind-before-spawn in tests`

### Task 3: Add mock_daemon_verify helper (#340)

**Crate**: `minibox-crux-plugin`
**File(s)**: `crates/minibox-crux-plugin/tests/integration.rs`
**Run**: `cargo test -p minibox-crux-plugin`

1. Add a new mock helper that captures and returns the request:

   ```rust
   use minibox_core::protocol::DaemonRequest;
   use tokio::sync::oneshot;

   /// Mock daemon that captures the incoming request and sends a canned
   /// response. Returns the deserialized DaemonRequest via the oneshot.
   async fn mock_daemon_verify(
       listener: UnixListener,
       response: DaemonResponse,
       tx: oneshot::Sender<DaemonRequest>,
   ) {
       let (stream, _) = listener.accept().await.expect("accept");
       let (read_half, mut write_half) = tokio::io::split(stream);
       let mut reader = BufReader::new(read_half);
       let mut line = String::new();
       reader.read_line(&mut line).await.expect("read request");

       let request: DaemonRequest =
           serde_json::from_str(line.trim()).expect("deserialize DaemonRequest");
       let _ = tx.send(request);

       let mut resp = serde_json::to_string(&response).expect("serialize");
       resp.push('\n');
       write_half.write_all(resp.as_bytes()).await.expect("write");
       write_half.flush().await.expect("flush");
   }
   ```

2. Add a test that verifies ps sends `DaemonRequest::List`:

   ```rust
   #[tokio::test]
   async fn invoke_ps_sends_list_request() {
       let tmp = TempDir::new().expect("tempdir");
       let socket_path = tmp.path().join("daemon.sock");
       let listener = UnixListener::bind(&socket_path).expect("bind");
       let (tx, rx) = oneshot::channel();
       tokio::spawn(mock_daemon_verify(
           listener,
           DaemonResponse::ContainerList { containers: vec![] },
           tx,
       ));
       let mut h = PluginHarness::spawn(&socket_path);

       let resp = h.invoke("minibox::container::ps", json!({})).await;
       assert_eq!(resp["status"], "InvokeOk");

       let req = rx.await.expect("request captured");
       assert!(
           matches!(req, DaemonRequest::List),
           "expected List, got: {req:?}"
       );

       h.shutdown().await;
   }
   ```

3. Verify:
   ```
   cargo test -p minibox-crux-plugin    -> all pass
   cargo clippy -p minibox-crux-plugin -- -D warnings  -> zero warnings
   ```

4. Commit: `test(crux-plugin): add mock_daemon_verify for request assertions`

### Task 4: Add mount round-trip integration test (#339)

**Crate**: `minibox-crux-plugin`
**File(s)**: `crates/minibox-crux-plugin/tests/integration.rs`
**Run**: `cargo test -p minibox-crux-plugin`

1. Add test using `mock_daemon_verify`:

   ```rust
   #[tokio::test]
   async fn invoke_run_with_mounts_sends_correct_bind_mounts() {
       let tmp = TempDir::new().expect("tempdir");
       let socket_path = tmp.path().join("daemon.sock");
       let listener = UnixListener::bind(&socket_path).expect("bind");
       let (tx, rx) = oneshot::channel();
       tokio::spawn(mock_daemon_verify(
           listener,
           DaemonResponse::ContainerCreated {
               id: "test-123".into(),
           },
           tx,
       ));
       let mut h = PluginHarness::spawn(&socket_path);

       let resp = h.invoke("minibox::container::run", json!({
           "image": "alpine:latest",
           "command": ["/bin/sh"],
           "mounts": [{
               "host_path": "/tmp/data",
               "container_path": "/data",
               "read_only": true
           }]
       })).await;
       assert_eq!(resp["status"], "InvokeOk");

       let req = rx.await.expect("request captured");
       match req {
           DaemonRequest::Run { mounts, .. } => {
               assert_eq!(mounts.len(), 1);
               assert_eq!(
                   mounts[0].host_path,
                   std::path::PathBuf::from("/tmp/data")
               );
               assert_eq!(
                   mounts[0].container_path,
                   std::path::PathBuf::from("/data")
               );
               assert!(mounts[0].read_only);
           }
           other => panic!("expected Run, got: {other:?}"),
       }

       h.shutdown().await;
   }
   ```

2. Verify:
   ```
   cargo test -p minibox-crux-plugin    -> all pass
   cargo clippy -p minibox-crux-plugin -- -D warnings  -> zero warnings
   ```

3. Commit: `test(crux-plugin): add mount round-trip integration test`

### Task 5: Add exec, build, and logs integration tests (#338)

**Crate**: `minibox-crux-plugin`
**File(s)**: `crates/minibox-crux-plugin/tests/integration.rs`
**Run**: `cargo test -p minibox-crux-plugin`

Note: `dispatch()` considers `ContainerStopped` and `Success` terminal but
NOT `BuildComplete` or `ExecStarted`. Streaming mocks must end with a
terminal variant.

1. Refactor `mock_daemon_multi` to accept a pre-bound `UnixListener`
   (should already be done in Task 2). Remove `#[allow(dead_code)]`.

2. Add exec test (ExecStarted + ContainerOutput + ContainerStopped):

   ```rust
   #[tokio::test]
   async fn invoke_exec_returns_streaming_output() {
       use minibox_core::protocol::OutputStreamKind;

       let tmp = TempDir::new().expect("tempdir");
       let socket_path = tmp.path().join("daemon.sock");
       let listener = UnixListener::bind(&socket_path).expect("bind");
       tokio::spawn(mock_daemon_multi(
           listener,
           vec![
               DaemonResponse::ExecStarted {
                   exec_id: "exec-1".into(),
               },
               DaemonResponse::ContainerOutput {
                   stream: OutputStreamKind::Stdout,
                   data: "aGVsbG8=".into(), // base64("hello")
               },
               DaemonResponse::ContainerStopped { exit_code: 0 },
           ],
       ));
       let mut h = PluginHarness::spawn(&socket_path);

       let resp = h.invoke("minibox::container::exec", json!({
           "id": "abc123",
           "command": ["/bin/echo", "hello"]
       })).await;
       assert_eq!(resp["status"], "InvokeOk");
       // Streaming: output is an array of all responses
       let output = &resp["data"];
       assert!(output.is_array(), "streaming output must be array");
       let arr = output.as_array().expect("array");
       assert_eq!(arr.len(), 3, "ExecStarted + ContainerOutput + ContainerStopped");

       h.shutdown().await;
   }
   ```

3. Add build test (BuildOutput + BuildComplete + Success as terminal):

   ```rust
   #[tokio::test]
   async fn invoke_build_returns_streaming_output() {
       let tmp = TempDir::new().expect("tempdir");
       let socket_path = tmp.path().join("daemon.sock");
       let listener = UnixListener::bind(&socket_path).expect("bind");
       tokio::spawn(mock_daemon_multi(
           listener,
           vec![
               DaemonResponse::BuildOutput {
                   step: 1,
                   total: 2,
                   line: "Step 1/2 : FROM alpine".into(),
               },
               DaemonResponse::BuildComplete {
                   image_id: "sha256:abc123".into(),
               },
               DaemonResponse::Success {
                   message: "build complete".into(),
               },
           ],
       ));
       let mut h = PluginHarness::spawn(&socket_path);

       let resp = h.invoke("minibox::image::build", json!({
           "context_path": "/tmp/ctx",
           "tag": "test:latest"
       })).await;
       assert_eq!(resp["status"], "InvokeOk");
       let output = &resp["data"];
       assert!(output.is_array(), "streaming output must be array");
       let arr = output.as_array().expect("array");
       assert_eq!(arr.len(), 3, "BuildOutput + BuildComplete + Success");

       h.shutdown().await;
   }
   ```

4. Add logs test (ContainerOutput + ContainerStopped):

   ```rust
   #[tokio::test]
   async fn invoke_logs_returns_streaming_output() {
       use minibox_core::protocol::OutputStreamKind;

       let tmp = TempDir::new().expect("tempdir");
       let socket_path = tmp.path().join("daemon.sock");
       let listener = UnixListener::bind(&socket_path).expect("bind");
       tokio::spawn(mock_daemon_multi(
           listener,
           vec![
               DaemonResponse::ContainerOutput {
                   stream: OutputStreamKind::Stdout,
                   data: "bG9nIGxpbmU=".into(),
               },
               DaemonResponse::ContainerStopped { exit_code: 0 },
           ],
       ));
       let mut h = PluginHarness::spawn(&socket_path);

       let resp = h.invoke("minibox::container::logs", json!({
           "id": "abc123"
       })).await;
       assert_eq!(resp["status"], "InvokeOk");
       let output = &resp["data"];
       assert!(output.is_array(), "streaming output must be array");
       let arr = output.as_array().expect("array");
       assert_eq!(arr.len(), 2, "ContainerOutput + ContainerStopped");

       h.shutdown().await;
   }
   ```

5. Verify:
   ```
   cargo test -p minibox-crux-plugin    -> all pass
   cargo clippy -p minibox-crux-plugin -- -D warnings  -> zero warnings
   ```

6. Commit: `test(crux-plugin): add exec, build, and logs streaming integration tests`

## Risks

- `BuildComplete` is NOT in `dispatch()`'s `is_terminal` match. The build
  test must end with `Success` as the terminal response, not
  `BuildComplete`. This is arguably a bug in dispatch -- file a follow-up
  issue if confirmed during implementation.
- `ContainerLogs` response from the real daemon may not end with
  `ContainerStopped` -- verify against the actual handler. If the daemon
  closes the stream instead, the test mock must also close the write half.
