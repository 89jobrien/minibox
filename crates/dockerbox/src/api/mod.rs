pub mod containers;
pub mod exec;
pub mod images;
pub mod networks;
pub mod system;
pub mod volumes;

use std::sync::Arc;

use axum::Router;

use crate::domain::ContainerRuntime;
use crate::infra::state::StateStore;

#[derive(Clone)]
pub struct AppState {
    pub runtime: Arc<dyn ContainerRuntime>,
    pub state: StateStore,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        // system
        .route("/_ping", axum::routing::get(system::ping))
        .route("/version", axum::routing::get(system::version))
        .route("/info", axum::routing::get(system::info))
        // images
        .route("/images/create", axum::routing::post(images::pull))
        .route("/images/{name}/json", axum::routing::get(images::inspect))
        // networks
        .route("/networks/create", axum::routing::post(networks::create))
        .route("/networks", axum::routing::get(networks::list))
        .route("/networks/{id}", axum::routing::get(networks::inspect))
        .route("/networks/{id}", axum::routing::delete(networks::remove))
        // volumes
        .route("/volumes/create", axum::routing::post(volumes::create))
        .route("/volumes/{name}", axum::routing::get(volumes::inspect))
        .route("/volumes/{name}", axum::routing::delete(volumes::remove))
        // containers
        .route(
            "/containers/create",
            axum::routing::post(containers::create),
        )
        .route(
            "/containers/{id}/start",
            axum::routing::post(containers::start),
        )
        .route(
            "/containers/{id}/json",
            axum::routing::get(containers::inspect),
        )
        .route("/containers/json", axum::routing::get(containers::list))
        .route(
            "/containers/{id}/logs",
            axum::routing::get(containers::logs),
        )
        .route(
            "/containers/{id}/stop",
            axum::routing::post(containers::stop),
        )
        .route(
            "/containers/{id}/wait",
            axum::routing::post(containers::wait),
        )
        .route(
            "/containers/{id}",
            axum::routing::delete(containers::remove),
        )
        // exec
        .route(
            "/containers/{id}/exec",
            axum::routing::post(exec::create),
        )
        .route("/exec/{id}/start", axum::routing::post(exec::start))
        .route("/exec/{id}/json", axum::routing::get(exec::inspect))
        .with_state(state)
}
