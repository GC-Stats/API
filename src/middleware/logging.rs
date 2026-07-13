use axum::{
    extract::{State, Request},
    middleware::Next,
    response::Response,
};
use std::sync::Arc;
use std::time::Instant;
use chrono::Utc;
use redis::AsyncCommands;
use serde::{Deserialize, Serialize};
use crate::AppState;

pub const LOG_BUFFER_KEY: &str = "apilog:buffer";

#[derive(Debug, Clone, Copy)]
pub struct ApiKeyId(pub u64);

#[derive(Debug, Serialize, Deserialize)]
pub struct LogEntry {
    pub api_key_id: Option<u64>,
    pub method: String,
    pub endpoint: String,
    pub status_code: u16,
    pub duration_ms: u32,
    pub user_agent: Option<String>,
    pub created_at: chrono::DateTime<Utc>,
}

pub async fn mw_request_logger(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let method = req.method().to_string();
    // Log the matched route pattern ("/v1/players/{id}") rather than the raw
    // path so endpoint stats aggregate correctly instead of exploding in
    // cardinality (one row per concrete id). Unmatched paths fall back to raw.
    let endpoint = req.extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|p| p.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());
    let user_agent = req.headers()
        .get("user-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

    let start = Instant::now();
    let res = next.run(req).await;
    let duration_ms = start.elapsed().as_millis() as u32;

    let api_key_id = res.extensions().get::<ApiKeyId>().map(|k| k.0);

    let entry = LogEntry {
        api_key_id,
        method,
        endpoint,
        status_code: res.status().as_u16(),
        duration_ms,
        user_agent,
        created_at: Utc::now(),
    };

    if let Ok(payload) = serde_json::to_string(&entry) {
        let mut redis = state.redis_local.clone();
        if let Err(err) = redis.lpush::<_, _, ()>(LOG_BUFFER_KEY, payload).await {
            tracing::warn!("Failed to push request log to local Redis: {:?}", err);
        }
    }

    res
}
