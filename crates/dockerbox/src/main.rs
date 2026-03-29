mod api;
mod domain;
mod infra;

use std::sync::Arc;
use tokio::net::UnixListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let minibox_socket =
        std::env::var("MINIBOX_SOCKET").unwrap_or_else(|_| "/run/minibox/miniboxd.sock".into());

    let socket_path = std::env::var("DOCKERBOX_SOCKET")
        .unwrap_or_else(|_| "/run/dockerbox/dockerbox.sock".into());

    // Ensure socket dir exists
    if let Some(parent) = std::path::Path::new(&socket_path).parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Remove stale socket
    let _ = std::fs::remove_file(&socket_path);

    let runtime = Arc::new(crate::infra::minibox::MiniboxAdapter::new(&minibox_socket));
    let state = crate::infra::state::StateStore::default();
    let app_state = crate::api::AppState { runtime, state };
    let router = crate::api::router(app_state);

    tracing::info!("dockerboxd listening on {}", socket_path);
    let listener = UnixListener::bind(&socket_path)?;
    axum::serve(listener, router).await?;
    Ok(())
}
