/*
    GC-Stats — API

    Authentication and rate-limiting middleware for the `/v1` API. Validates
    the `x-api-key` header against MariaDB (cached in Redis) and enforces a
    sliding-window rate limit per key.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use axum::{
    extract::{State, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use crate::AppState;
use crate::models::auth::{hash_api_key, ApiKey};

use redis::{AsyncCommands, Script};

static RATE_LIMIT_SCRIPT: &str = r#"
    local key    = KEYS[1]
    local now    = tonumber(ARGV[1])
    local window = tonumber(ARGV[2])
    local limit  = tonumber(ARGV[3])

    redis.call('ZREMRANGEBYSCORE', key, '-inf', now - window)
    local count = redis.call('ZCARD', key)

    if count >= limit then
        return -1
    end

    redis.call('ZADD', key, now, now .. '-' .. math.random(1, 1000000))
    redis.call('PEXPIRE', key, window)

    return count + 1
"#;

/// Sliding-window rate limit check backed by Redis. Returns `Ok(false)` when
/// the caller identified by `key` has exceeded `limit` events per `window_ms`.
pub async fn check_rate_limit(
    redis: &mut redis::aio::ConnectionManager,
    key: &str,
    window_ms: i64,
    limit: i64,
) -> Result<bool, redis::RedisError> {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;

    let result: i64 = Script::new(RATE_LIMIT_SCRIPT)
        .key(key)
        .arg(now_ms)
        .arg(window_ms)
        .arg(limit)
        .invoke_async(redis)
        .await?;

    Ok(result != -1)
}

pub async fn mw_rate_limiter(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(api_key_hex) = req.headers()
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
        .map(|v| v.to_string())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let key_hash = hash_api_key(&api_key_hex);

    let cache_key = format!("apikey_cache:{}", key_hash);
    let mut redis = state.redis.clone();

    let cached: Option<ApiKey> = redis
        .get::<_, Option<String>>(&cache_key)
        .await
        .ok()
        .flatten()
        .and_then(|json| serde_json::from_str(&json).ok());

    let key_info = if let Some(key) = cached {
        key
    } else {
        let key = match sqlx::query_as::<_, ApiKey>(
            "SELECT id, client_name, rate_limit, is_active FROM api_key WHERE key_hash = ? AND is_active = 1"
        )
            .bind(&key_hash)
            .fetch_optional(&state.db)
            .await
        {
            Ok(Some(k)) => k,
            Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        };
        if let Ok(json) = serde_json::to_string(&key) {
            let _: Result<(), _> = redis.set_ex(&cache_key, json, 60u64).await;
        }
        key
    };
    let redis_key = format!("ratelimit:{}", key_hash);

    let allowed = match check_rate_limit(&mut redis, &redis_key, 60_000, key_info.rate_limit as i64).await {
        Ok(allowed) => allowed,
        Err(err) => {
            tracing::error!("Redis error in rate limiter: {:?}", err);
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    if !allowed {
        let mut res = StatusCode::TOO_MANY_REQUESTS.into_response();
        res.extensions_mut().insert(crate::middleware::logging::ApiKeyId(key_info.id));
        return res;
    }

    let mut res = next.run(req).await;
    res.extensions_mut().insert(crate::middleware::logging::ApiKeyId(key_info.id));

    res
}