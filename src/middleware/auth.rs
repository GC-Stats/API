/*
    GC-Stats — API

    Authentication and rate-limiting middleware for the `/v1` API (validates
    the `x-api-key` header against MariaDB, cached in Redis, and enforces a
    sliding-window rate limit per key), plus the HMAC-based internal auth
    middleware guarding internal service-to-service routes.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use axum::{
    body::Body,
    extract::{State, Request},
    http::StatusCode,
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::sync::Arc;
use crate::AppState;
use crate::models::auth::{hash_api_key, ApiKey};

use hmac::{Hmac, Mac};
use redis::{AsyncCommands, Script};
use sha2::Sha256;

const MAX_TIMESTAMP_SKEW_SECS: i64 = 300;

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
            .fetch_optional(&state.db_read)
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

/// Verifies `X-Internal-Signature`/`X-Internal-Timestamp`, mirroring the
/// scheme the DiscordBot uses to call Laravel: HMAC-SHA256 over
/// `{timestamp}.{method}.{path}.{body}`, hex-encoded, keyed by
/// `INTERNAL_API_SECRET`. Used for trusted service-to-service routes
/// (`/internal/*`), not end-user auth.
pub async fn mw_internal_auth(
    State(state): State<Arc<AppState>>,
    req: Request,
    next: Next,
) -> Response {
    let Some(timestamp) = req.headers()
        .get("x-internal-timestamp")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let Some(signature) = req.headers()
        .get("x-internal-signature")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
    else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    let Ok(timestamp_secs) = timestamp.parse::<i64>() else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    if !state.internal_auth_skip_timestamp_check {
        let now_secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;

        if (now_secs - timestamp_secs).abs() > MAX_TIMESTAMP_SKEW_SECS {
            return StatusCode::UNAUTHORIZED.into_response();
        }
    }

    let method = req.method().to_string();
    // `req.uri()` has the `/internal` prefix stripped by `Router::nest` by the
    // time this layer (applied to the nested sub-router) runs. `OriginalUri`
    // is what `nest` preserves the pre-rewrite path on, and it's what the
    // caller actually signed.
    let path = req.extensions()
        .get::<axum::extract::OriginalUri>()
        .map(|uri| uri.0.path().to_string())
        .unwrap_or_else(|| req.uri().path().to_string());

    let (parts, body) = req.into_parts();
    let Ok(body_bytes) = axum::body::to_bytes(body, 1024 * 1024).await else {
        return StatusCode::BAD_REQUEST.into_response();
    };
    let body_str = String::from_utf8_lossy(&body_bytes);

    let payload = format!("{}.{}.{}.{}", timestamp, method, path, body_str);

    let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(state.internal_api_secret.as_bytes()) else {
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    mac.update(payload.as_bytes());
    let expected_signature = hex::encode(mac.finalize().into_bytes());

    let signature_valid = signature.len() == expected_signature.len()
        && signature.bytes().zip(expected_signature.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0;

    if !signature_valid {
        return StatusCode::UNAUTHORIZED.into_response();
    }

    let req = Request::from_parts(parts, Body::from(body_bytes));
    next.run(req).await
}