/*
    GC-Stats — API

    Health check endpoint used for uptime monitoring.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use axum::{Json};
use serde_json::{json, Value};

pub async fn health_check() -> Json<Value> {
    let version = std::env::var("APP_VERSION").unwrap_or_else(|_| "dev".to_string());

    Json(json!({
        "status": "ok",
        "version": version
    }))
}