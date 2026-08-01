/*
    GC-Stats — API

    Declares the routes module and assembles the versioned `/v1` API router
    from the teams, players, tournaments, matches and map routers.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use std::sync::Arc;
use axum::Router;
use crate::AppState;

pub mod health;
pub mod teams;
pub mod players;
pub mod tournaments;
pub mod matches;
pub mod map;
pub mod internal;

pub fn api_router_v1() -> Router<Arc<AppState>> {
    Router::new()
        .nest("/teams", teams::router())
        .nest("/players", players::router())
        .nest("/tournaments", tournaments::router())
        .nest("/matches", matches::router())
        .nest("/map", map::router())
}

pub fn api_router_v2() -> Router<Arc<AppState>> {
    Router::new()
        .nest("/matches", matches::router_v2())
}