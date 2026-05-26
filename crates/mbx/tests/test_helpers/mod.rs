use std::path::Path;

/// Poll until a Unix socket path exists on disk, with a deadline.
pub async fn wait_for_socket(path: &Path, timeout_ms: u64) {
    let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_millis(timeout_ms);
    while !path.exists() {
        if tokio::time::Instant::now() >= deadline {
            panic!("socket {:?} did not appear within {}ms", path, timeout_ms);
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(5)).await;
    }
}
