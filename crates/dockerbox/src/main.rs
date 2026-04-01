use dockerbox::api;
use dockerbox::infra;

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

    let listener = UnixListener::bind(&socket_path)?;

    // SECURITY: Set socket permissions.
    //
    // Default: 0o660 — root-owned, group-accessible. Matches the Docker daemon
    // convention where members of the `docker` group can connect without sudo.
    // Set DOCKERBOX_SOCKET_GROUP to the group that should own the socket (e.g.
    // "docker"), and DOCKERBOX_SOCKET_MODE to override the octal permission bits.
    //
    // The upstream miniboxd socket is separately protected by SO_PEERCRED (UID 0
    // only). dockerbox inherits that gate for any operation that reaches miniboxd.
    {
        use std::os::unix::fs::PermissionsExt;

        let sock_path = std::path::Path::new(&socket_path);

        let mut mode = 0o660u32;
        if let Ok(mode_str) = std::env::var("DOCKERBOX_SOCKET_MODE") {
            let mode_str = mode_str.trim();
            let mode_str = mode_str.strip_prefix("0o").unwrap_or(mode_str);
            match u32::from_str_radix(mode_str, 8) {
                Ok(parsed) => mode = parsed,
                Err(err) => tracing::warn!("invalid DOCKERBOX_SOCKET_MODE={mode_str}: {err}"),
            }
        }

        if let Ok(group_name) = std::env::var("DOCKERBOX_SOCKET_GROUP") {
            let group_name = group_name.trim().to_owned();
            if !group_name.is_empty() {
                match nix::unistd::Group::from_name(&group_name) {
                    Ok(Some(group)) => {
                        nix::unistd::chown(sock_path, None, Some(group.gid))
                            .map_err(|e| anyhow::anyhow!("chown socket to {group_name}: {e}"))?;
                        tracing::info!(group = %group_name, "socket group set");
                    }
                    Ok(None) => {
                        tracing::warn!(group = %group_name, "DOCKERBOX_SOCKET_GROUP not found")
                    }
                    Err(e) => {
                        tracing::warn!(group = %group_name, error = %e, "group lookup failed")
                    }
                }
            }
        }

        let metadata = std::fs::metadata(sock_path)?;
        let mut permissions = metadata.permissions();
        permissions.set_mode(mode);
        std::fs::set_permissions(sock_path, permissions)
            .map_err(|e| anyhow::anyhow!("set socket permissions {mode:04o}: {e}"))?;
        tracing::info!(mode = format!("{mode:04o}"), "socket permissions set");
    }

    let runtime = Arc::new(crate::infra::minibox::MiniboxAdapter::new(&minibox_socket));
    let state = crate::infra::state::StateStore::default();
    let app_state = crate::api::AppState { runtime, state };
    let router = crate::api::router(app_state);

    tracing::info!("dockerboxd listening on {}", socket_path);
    axum::serve(listener, router).await?;
    Ok(())
}
