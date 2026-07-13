use std::sync::Arc;
use axum::Router;
use crate::AppState;

pub mod health;
pub mod dashboard;
pub mod teams;
pub mod players;
pub mod tournaments;
pub mod matches;
pub mod map;

pub fn api_router_v1() -> Router<Arc<AppState>> {
    Router::new()
        .nest("/teams", teams::router())
        .nest("/players", players::router())
        .nest("/tournaments", tournaments::router())
        .nest("/matches", matches::router())
        .nest("/map", map::router())
}