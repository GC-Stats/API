/*
    GC-Stats — API

    Routes for the client-facing usage dashboard: login/logout via API key,
    request statistics (summary, status breakdown, top endpoints,
    time-bucketed chart), recent logs, and the paginated request history
    page.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use std::net::SocketAddr;
use std::sync::Arc;
use askama::Template;
use axum::{
    extract::{ConnectInfo, Query, State},
    http::{header, HeaderMap, StatusCode},
    response::{Html, IntoResponse, Json, Redirect, Response},
    routing::{get, post},
    Form, Router,
};
use chrono::{NaiveDateTime, TimeZone, Utc};
use cookie::{Cookie, CookieJar, SameSite};
use serde::{Deserialize, Serialize};
use sqlx::mysql::MySql;
use sqlx::{FromRow, QueryBuilder};

use crate::middleware::auth::check_rate_limit;
use crate::models::auth::{hash_api_key, ApiKey};
use crate::util::escape_like;
use crate::AppState;

const SESSION_COOKIE: &str = "gc_dash";
const SESSION_MAX_AGE: cookie::time::Duration = cookie::time::Duration::hours(24);
const BUCKET_COUNT: i64 = 24;
const RECENT_LOGS_PAGE_SIZE: i64 = 50;
const HISTORY_PAGE_SIZE: i64 = 50;

// Brute-force protection on the login form: max attempts per IP per window.
const LOGIN_ATTEMPT_LIMIT: i64 = 5;
const LOGIN_WINDOW_MS: i64 = 60_000;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/dashboard", get(dashboard_page))
        .route("/dashboard/login", post(login))
        .route("/dashboard/logout", post(logout))
        .route("/dashboard/recent-logs", get(recent_logs_json))
        .route("/dashboard/history", get(history_page))
}

#[derive(Deserialize)]
pub struct LoginForm {
    api_key: String,
}

#[derive(Deserialize)]
struct DashboardQuery {
    bucket: Option<String>,
    sort: Option<String>,
}

#[derive(Deserialize)]
struct RecentLogsQuery {
    offset: Option<i64>,
    sort: Option<String>,
}

#[derive(Template)]
#[template(path = "dashboard_login.html")]
struct LoginTemplate {
    error: Option<String>,
}

#[derive(FromRow)]
struct Summary {
    total: i64,
    avg_duration: f64,
    max_duration: i64,
    error_count: i64,
}

#[derive(FromRow)]
struct StatusBreakdown {
    success: i64,
    not_found: i64,
    rate_limited: i64,
    client_error: i64,
    server_error: i64,
}

#[derive(FromRow)]
struct EndpointStat {
    method: String,
    endpoint: String,
    count: i64,
    avg_duration: f64,
}

struct EndpointStatDisplay {
    method: String,
    method_class: String,
    endpoint: String,
    count: i64,
    avg_duration_ms: String,
}

#[derive(FromRow)]
struct BucketCount {
    bucket_index: i64,
    count: i64,
}

#[derive(FromRow)]
struct RecentLog {
    method: String,
    endpoint: String,
    status_code: i32,
    duration_ms: i32,
    user_agent: Option<String>,
    created_at: chrono::NaiveDateTime,
}

struct BucketCountDisplay {
    label: String,
    cy: f64,
    cx: f64,
    count: i64,
    hit_x: f64,
    hit_width: f64,
}

/// Render-ready log row, shared by the HTML templates and the JSON endpoint.
#[derive(Serialize)]
struct RecentLogView {
    method: String,
    method_class: String,
    endpoint: String,
    status_code: i32,
    status_class: String,
    duration_ms: i32,
    user_agent: String,
    created_at: String,
}

impl From<RecentLog> for RecentLogView {
    fn from(log: RecentLog) -> Self {
        Self {
            method_class: method_class(&log.method),
            method: log.method,
            endpoint: log.endpoint,
            status_code: log.status_code,
            status_class: status_class(log.status_code),
            duration_ms: log.duration_ms,
            user_agent: log.user_agent.unwrap_or_else(|| "-".to_string()),
            created_at: log.created_at.format("%Y-%m-%d %H:%M:%S").to_string(),
        }
    }
}

#[derive(Template)]
#[template(path = "dashboard.html")]
struct DashboardTemplate {
    nav_active: &'static str,
    key_preview: String,
    rate_limit: i32,
    total_requests: i64,
    avg_duration_ms: String,
    max_duration_ms: i64,
    error_rate: String,
    success_count: i64,
    not_found_count: i64,
    rate_limited_count: i64,
    client_error_count: i64,
    server_error_count: i64,
    top_endpoints: Vec<EndpointStatDisplay>,
    bucket_counts: Vec<BucketCountDisplay>,
    chart_points: String,
    max_bucket_count: i64,
    bucket_selected: String,
    sort_selected: String,
    recent_logs: Vec<RecentLogView>,
    recent_logs_has_more: bool,
    recent_logs_offset: i64,
}

fn read_cookie_jar(headers: &HeaderMap) -> CookieJar {
    let mut jar = CookieJar::new();

    if let Some(raw) = headers.get(header::COOKIE).and_then(|v| v.to_str().ok()) {
        for part in raw.split(';') {
            if let Ok(cookie) = Cookie::parse_encoded(part.trim().to_owned()) {
                jar.add_original(cookie);
            }
        }
    }

    jar
}

fn apply_cookie_jar(jar: &CookieJar, mut res: Response) -> Response {
    for cookie in jar.delta() {
        if let Ok(value) = cookie.encoded().to_string().parse() {
            res.headers_mut().append(header::SET_COOKIE, value);
        }
    }
    res
}

fn render<T: Template>(tpl: &T) -> Response {
    match tpl.render() {
        Ok(html) => Html(html).into_response(),
        Err(err) => {
            tracing::error!("Template render error: {:?}", err);
            (StatusCode::INTERNAL_SERVER_ERROR, "Template error").into_response()
        }
    }
}

fn session_cookie(api_key_id: u64) -> Cookie<'static> {
    let secure = std::env::var("APP_ENV").map(|v| v == "production").unwrap_or(false);

    Cookie::build((SESSION_COOKIE, api_key_id.to_string()))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(secure)
        .max_age(SESSION_MAX_AGE)
        .build()
}

fn expired_session_cookie() -> Cookie<'static> {
    let secure = std::env::var("APP_ENV").map(|v| v == "production").unwrap_or(false);

    Cookie::build((SESSION_COOKIE, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(secure)
        .max_age(cookie::time::Duration::ZERO)
        .build()
}

fn method_class(method: &str) -> String {
    match method.to_uppercase().as_str() {
        "GET" => "text-win border-win/40",
        "POST" => "text-brand border-brand/40",
        "PUT" => "text-[#4aa8ff] border-[#4aa8ff]/40",
        "PATCH" => "text-[#c084fc] border-[#c084fc]/40",
        "DELETE" => "text-loss border-loss/40",
        _ => "text-secondary border-subtle",
    }.to_string()
}

fn status_class(status: i32) -> String {
    match status {
        200..=299 => "text-win font-semibold",
        300..=399 | 404 | 429 => "text-brand font-semibold",
        400..=499 => "text-loss font-semibold",
        _ => "text-loss font-semibold",
    }.to_string()
}

/// Returns the bucket width in seconds for a given selector, defaulting to 1h.
fn bucket_seconds(bucket: &str) -> i64 {
    match bucket {
        "6h" => 6 * 3600,
        "12h" => 12 * 3600,
        "24h" => 24 * 3600,
        _ => 3600,
    }
}

fn bucket_label(bucket: &str, bucket_index: i64, seconds: i64) -> String {
    let dt = Utc.timestamp_opt(bucket_index * seconds, 0).single().unwrap_or_else(Utc::now);
    match bucket {
        "6h" | "12h" => dt.format("%d/%m %Hh").to_string(),
        "24h" => dt.format("%d/%m").to_string(),
        _ => dt.format("%H:%M").to_string(),
    }
}

fn sort_order_clause(sort: &str) -> &'static str {
    match sort {
        "status" => "ORDER BY status_code DESC, id DESC",
        _ => "ORDER BY id DESC",
    }
}

fn current_api_key_id(jar: &CookieJar, state: &AppState) -> Option<u64> {
    jar.signed(&state.cookie_key)
        .get(SESSION_COOKIE)
        .and_then(|c| c.value().parse::<u64>().ok())
}

async fn fetch_active_key(state: &AppState, api_key_id: u64) -> Result<Option<ApiKey>, sqlx::Error> {
    sqlx::query_as::<_, ApiKey>(
        "SELECT id, client_name, rate_limit, is_active FROM api_key WHERE id = ? AND is_active = 1"
    )
        .bind(api_key_id)
        .fetch_optional(&state.db)
        .await
}

/// Client IP for rate limiting: first entry of X-Forwarded-For when running
/// behind the reverse proxy, otherwise the socket peer address.
fn client_ip(headers: &HeaderMap, fallback: SocketAddr) -> String {
    headers.get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| fallback.ip().to_string())
}

async fn dashboard_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<DashboardQuery>,
) -> Response {
    let jar = read_cookie_jar(&headers);
    let Some(api_key_id) = current_api_key_id(&jar, &state) else {
        return render(&LoginTemplate { error: None });
    };

    let api_key = match fetch_active_key(&state, api_key_id).await {
        Ok(Some(key)) => key,
        Ok(None) => return render(&LoginTemplate {
            error: Some("This API key is no longer active.".to_string()),
        }),
        Err(err) => {
            tracing::error!("DB error fetching api_key: {:?}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let bucket = query.bucket.unwrap_or_else(|| "1h".to_string());
    let bucket = if matches!(bucket.as_str(), "1h" | "6h" | "12h" | "24h") { bucket } else { "1h".to_string() };

    let sort = query.sort.unwrap_or_else(|| "date".to_string());
    let sort = if matches!(sort.as_str(), "date" | "status") { sort } else { "date".to_string() };

    let seconds = bucket_seconds(&bucket);
    let now_bucket_index = Utc::now().timestamp() / seconds;
    let window_start_index = now_bucket_index - (BUCKET_COUNT - 1);
    let window_start = Utc.timestamp_opt(window_start_index * seconds, 0).single().unwrap_or_else(Utc::now);

    let summary_query = sqlx::query_as::<_, Summary>(
        r#"
        SELECT
            COUNT(*) AS total,
            COALESCE(CAST(AVG(duration_ms) AS DOUBLE), 0) AS avg_duration,
            CAST(COALESCE(MAX(duration_ms), 0) AS SIGNED) AS max_duration,
            CAST(COALESCE(SUM(CASE WHEN status_code >= 400 THEN 1 ELSE 0 END), 0) AS SIGNED) AS error_count
        FROM api_request_log
        WHERE api_key_id = ?
        "#
    )
        .bind(api_key_id)
        .fetch_one(&state.db);

    let breakdown_query = sqlx::query_as::<_, StatusBreakdown>(
        r#"
        SELECT
            CAST(COALESCE(SUM(CASE WHEN status_code < 300 THEN 1 ELSE 0 END), 0) AS SIGNED) AS success,
            CAST(COALESCE(SUM(CASE WHEN status_code = 404 THEN 1 ELSE 0 END), 0) AS SIGNED) AS not_found,
            CAST(COALESCE(SUM(CASE WHEN status_code = 429 THEN 1 ELSE 0 END), 0) AS SIGNED) AS rate_limited,
            CAST(COALESCE(SUM(CASE WHEN status_code >= 400 AND status_code < 500 AND status_code NOT IN (404, 429) THEN 1 ELSE 0 END), 0) AS SIGNED) AS client_error,
            CAST(COALESCE(SUM(CASE WHEN status_code >= 500 THEN 1 ELSE 0 END), 0) AS SIGNED) AS server_error
        FROM api_request_log
        WHERE api_key_id = ?
        "#
    )
        .bind(api_key_id)
        .fetch_one(&state.db);

    let top_endpoints_query = sqlx::query_as::<_, EndpointStat>(
        r#"
        SELECT
            method,
            endpoint,
            COUNT(*) AS count,
            CAST(AVG(duration_ms) AS DOUBLE) AS avg_duration
        FROM api_request_log
        WHERE api_key_id = ?
        GROUP BY method, endpoint
        ORDER BY count DESC
        LIMIT 15
        "#
    )
        .bind(api_key_id)
        .fetch_all(&state.db);

    let bucket_counts_query = sqlx::query_as::<_, BucketCount>(
        r#"
        SELECT
            CAST(FLOOR(UNIX_TIMESTAMP(created_at) / ?) AS SIGNED) AS bucket_index,
            COUNT(*) AS count
        FROM api_request_log
        WHERE api_key_id = ? AND created_at >= ?
        GROUP BY bucket_index
        ORDER BY bucket_index
        "#
    )
        .bind(seconds)
        .bind(api_key_id)
        .bind(window_start.naive_utc())
        .fetch_all(&state.db);

    let mut recent_logs_qb: QueryBuilder<MySql> = QueryBuilder::new(
        "SELECT method, endpoint, CAST(status_code AS SIGNED) AS status_code, CAST(duration_ms AS SIGNED) AS duration_ms, user_agent, created_at FROM api_request_log WHERE api_key_id = "
    );
    recent_logs_qb
        .push_bind(api_key_id)
        .push(format!(" {} LIMIT {}", sort_order_clause(&sort), RECENT_LOGS_PAGE_SIZE + 1));
    let recent_logs_query = recent_logs_qb.build_query_as::<RecentLog>().fetch_all(&state.db);

    let (summary, breakdown, top_endpoints, bucket_counts, recent_logs) = tokio::join!(
        summary_query,
        breakdown_query,
        top_endpoints_query,
        bucket_counts_query,
        recent_logs_query,
    );

    let (summary, breakdown, top_endpoints, bucket_counts, recent_logs) =
        match (summary, breakdown, top_endpoints, bucket_counts, recent_logs) {
            (Ok(s), Ok(b), Ok(t), Ok(d), Ok(r)) => (s, b, t, d, r),
            _ => {
                tracing::error!("DB error fetching dashboard stats");
                return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
            }
        };

    let error_rate = if summary.total > 0 {
        (summary.error_count as f64 / summary.total as f64) * 100.0
    } else {
        0.0
    };

    let top_endpoints = top_endpoints.into_iter()
        .map(|ep| EndpointStatDisplay {
            method_class: method_class(&ep.method),
            method: ep.method,
            endpoint: ep.endpoint,
            count: ep.count,
            avg_duration_ms: format!("{:.1}", ep.avg_duration),
        })
        .collect();

    let mut counts_by_bucket = std::collections::HashMap::new();
    for b in bucket_counts {
        counts_by_bucket.insert(b.bucket_index, b.count);
    }

    let max_bucket_count = counts_by_bucket.values().copied().max().unwrap_or(0).max(1);
    let chart_step = if BUCKET_COUNT > 1 { 100.0 / (BUCKET_COUNT - 1) as f64 } else { 0.0 };
    let hit_width = if chart_step > 0.0 { chart_step } else { 104.0 };
    let bucket_counts: Vec<BucketCountDisplay> = (0..BUCKET_COUNT)
        .map(|i| {
            let index = window_start_index + i;
            let count = counts_by_bucket.get(&index).copied().unwrap_or(0);
            let bar_height = ((count as f64 / max_bucket_count as f64) * 88.0).max(2.0);
            let cx = i as f64 * chart_step;
            BucketCountDisplay {
                label: bucket_label(&bucket, index, seconds),
                cx,
                cy: 96.0 - bar_height,
                count,
                hit_x: cx - hit_width / 2.0,
                hit_width,
            }
        })
        .collect();

    let chart_points = bucket_counts.iter()
        .map(|b| format!("{:.2},{:.2}", b.cx, b.cy))
        .collect::<Vec<_>>()
        .join(" ");

    let recent_logs_has_more = recent_logs.len() as i64 > RECENT_LOGS_PAGE_SIZE;
    let recent_logs = recent_logs.into_iter()
        .take(RECENT_LOGS_PAGE_SIZE as usize)
        .map(RecentLogView::from)
        .collect();

    render(&DashboardTemplate {
        nav_active: "dashboard",
        key_preview: api_key.client_name.clone(),
        rate_limit: api_key.rate_limit,
        total_requests: summary.total,
        avg_duration_ms: format!("{:.1}", summary.avg_duration),
        max_duration_ms: summary.max_duration,
        error_rate: format!("{:.1}", error_rate),
        success_count: breakdown.success,
        not_found_count: breakdown.not_found,
        rate_limited_count: breakdown.rate_limited,
        client_error_count: breakdown.client_error,
        server_error_count: breakdown.server_error,
        top_endpoints,
        bucket_counts,
        chart_points,
        max_bucket_count,
        bucket_selected: bucket,
        sort_selected: sort,
        recent_logs,
        recent_logs_has_more,
        recent_logs_offset: RECENT_LOGS_PAGE_SIZE,
    })
}

async fn recent_logs_json(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<RecentLogsQuery>,
) -> Response {
    let jar = read_cookie_jar(&headers);
    let Some(api_key_id) = current_api_key_id(&jar, &state) else {
        return StatusCode::UNAUTHORIZED.into_response();
    };

    // Reject sessions whose API key has been deactivated since login.
    match fetch_active_key(&state, api_key_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(err) => {
            tracing::error!("DB error fetching api_key: {:?}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    }

    let sort = query.sort.unwrap_or_else(|| "date".to_string());
    let sort = if matches!(sort.as_str(), "date" | "status") { sort } else { "date".to_string() };
    let offset = query.offset.unwrap_or(0).max(0);

    let mut recent_logs_qb: QueryBuilder<MySql> = QueryBuilder::new(
        "SELECT method, endpoint, CAST(status_code AS SIGNED) AS status_code, CAST(duration_ms AS SIGNED) AS duration_ms, user_agent, created_at FROM api_request_log WHERE api_key_id = "
    );
    recent_logs_qb
        .push_bind(api_key_id)
        .push(format!(" {} LIMIT {}", sort_order_clause(&sort), RECENT_LOGS_PAGE_SIZE))
        .push(" OFFSET ")
        .push_bind(offset);
    let recent_logs = recent_logs_qb.build_query_as::<RecentLog>().fetch_all(&state.db).await;

    let recent_logs = match recent_logs {
        Ok(logs) => logs,
        Err(err) => {
            tracing::error!("DB error fetching recent logs: {:?}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let logs: Vec<RecentLogView> = recent_logs.into_iter()
        .map(RecentLogView::from)
        .collect();

    Json(logs).into_response()
}

async fn login(
    State(state): State<Arc<AppState>>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Form(form): Form<LoginForm>,
) -> Response {
    // The dashboard routes sit outside the /v1 API-key rate limiter, so the
    // login form needs its own per-IP brute-force protection.
    let ip = client_ip(&headers, addr);
    let mut redis = state.redis.clone();
    let rl_key = format!("ratelimit:dashboard_login:{}", ip);

    match check_rate_limit(&mut redis, &rl_key, LOGIN_WINDOW_MS, LOGIN_ATTEMPT_LIMIT).await {
        Ok(true) => {}
        Ok(false) => {
            tracing::warn!(ip = %ip, "Dashboard login rate limit exceeded");
            return (
                StatusCode::TOO_MANY_REQUESTS,
                render(&LoginTemplate { error: Some("Too many attempts. Try again in a minute.".to_string()) }),
            ).into_response();
        }
        Err(err) => {
            tracing::error!("Redis error on login rate limit: {:?}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    }

    let key_hash = hash_api_key(&form.api_key);
    let key = sqlx::query_as::<_, ApiKey>(
        "SELECT id, client_name, rate_limit, is_active FROM api_key WHERE key_hash = ? AND is_active = 1"
    )
        .bind(&key_hash)
        .fetch_optional(&state.db)
        .await;

    match key {
        Ok(Some(key)) => {
            let mut jar = read_cookie_jar(&headers);
            jar.signed_mut(&state.cookie_key).add(session_cookie(key.id));
            apply_cookie_jar(&jar, Redirect::to("/dashboard").into_response())
        }
        Ok(None) => {
            tracing::warn!(ip = %ip, "Failed dashboard login attempt");
            (
                StatusCode::UNAUTHORIZED,
                render(&LoginTemplate { error: Some("Invalid API key.".to_string()) }),
            ).into_response()
        }
        Err(err) => {
            tracing::error!("DB error during dashboard login: {:?}", err);
            (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response()
        }
    }
}

async fn logout() -> Response {
    let mut res = Redirect::to("/dashboard").into_response();
    if let Ok(value) = expired_session_cookie().encoded().to_string().parse() {
        res.headers_mut().append(header::SET_COOKIE, value);
    }
    res
}

#[derive(Deserialize)]
struct HistoryQuery {
    method: Option<String>,
    status: Option<String>,
    endpoint: Option<String>,
    date_from: Option<String>,
    date_to: Option<String>,
    page: Option<i64>,
}

struct HistoryFilters {
    method: Option<String>,
    status: Option<i32>,
    endpoint: Option<String>,
    date_from: Option<NaiveDateTime>,
    date_to: Option<NaiveDateTime>,
}

fn push_history_filters(qb: &mut QueryBuilder<MySql>, filters: &HistoryFilters) {
    if let Some(method) = &filters.method {
        qb.push(" AND method = ").push_bind(method.clone());
    }
    if let Some(status) = filters.status {
        qb.push(" AND status_code = ").push_bind(status);
    }
    if let Some(endpoint) = &filters.endpoint {
        qb.push(" AND endpoint LIKE ").push_bind(format!("%{}%", escape_like(endpoint)));
    }
    if let Some(from) = filters.date_from {
        qb.push(" AND created_at >= ").push_bind(from);
    }
    if let Some(to) = filters.date_to {
        qb.push(" AND created_at <= ").push_bind(to);
    }
}

#[derive(Template)]
#[template(path = "dashboard_history.html")]
struct HistoryTemplate {
    nav_active: &'static str,
    key_preview: String,
    rate_limit: i32,
    method_selected: String,
    status_value: String,
    endpoint_value: String,
    date_from_value: String,
    date_to_value: String,
    logs: Vec<RecentLogView>,
    page: i64,
    total_pages: i64,
    total: i64,
    has_prev: bool,
    has_next: bool,
}

async fn history_page(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Query(query): Query<HistoryQuery>,
) -> Response {
    let jar = read_cookie_jar(&headers);
    let Some(api_key_id) = current_api_key_id(&jar, &state) else {
        return render(&LoginTemplate { error: None });
    };

    let api_key = match fetch_active_key(&state, api_key_id).await {
        Ok(Some(key)) => key,
        Ok(None) => return render(&LoginTemplate {
            error: Some("This API key is no longer active.".to_string()),
        }),
        Err(err) => {
            tracing::error!("DB error fetching api_key: {:?}", err);
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let method_selected = query.method.clone().unwrap_or_default();
    let method_filter = query.method
        .filter(|m| !m.is_empty())
        .map(|m| m.to_uppercase());

    let status_value = query.status.clone().unwrap_or_default();
    let status_filter = query.status
        .filter(|s| !s.is_empty())
        .and_then(|s| s.parse::<i32>().ok());

    let endpoint_value = query.endpoint.clone().unwrap_or_default();
    let endpoint_filter = query.endpoint
        .filter(|e| !e.trim().is_empty())
        .map(|e| e.trim().to_string());

    let date_from_value = query.date_from.clone().unwrap_or_default();
    let date_from_filter = query.date_from
        .as_deref()
        .filter(|d| !d.is_empty())
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(0, 0, 0));

    let date_to_value = query.date_to.clone().unwrap_or_default();
    let date_to_filter = query.date_to
        .as_deref()
        .filter(|d| !d.is_empty())
        .and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
        .and_then(|d| d.and_hms_opt(23, 59, 59));

    let filters = HistoryFilters {
        method: method_filter,
        status: status_filter,
        endpoint: endpoint_filter,
        date_from: date_from_filter,
        date_to: date_to_filter,
    };

    let page = query.page.unwrap_or(1).max(1);
    let offset = (page - 1) * HISTORY_PAGE_SIZE;

    let mut count_qb: QueryBuilder<MySql> = QueryBuilder::new("SELECT COUNT(*) FROM api_request_log WHERE api_key_id = ");
    count_qb.push_bind(api_key_id);
    push_history_filters(&mut count_qb, &filters);

    let total_query = count_qb.build_query_scalar::<i64>().fetch_one(&state.db);

    let mut data_qb: QueryBuilder<MySql> = QueryBuilder::new(
        "SELECT method, endpoint, CAST(status_code AS SIGNED) AS status_code, CAST(duration_ms AS SIGNED) AS duration_ms, user_agent, created_at FROM api_request_log WHERE api_key_id = "
    );
    data_qb.push_bind(api_key_id);
    push_history_filters(&mut data_qb, &filters);
    data_qb.push(" ORDER BY id DESC LIMIT ").push_bind(HISTORY_PAGE_SIZE).push(" OFFSET ").push_bind(offset);

    let logs_query = data_qb.build_query_as::<RecentLog>().fetch_all(&state.db);

    let (total, logs) = tokio::join!(total_query, logs_query);

    let (total, logs) = match (total, logs) {
        (Ok(t), Ok(l)) => (t, l),
        _ => {
            tracing::error!("DB error fetching request history");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Database error").into_response();
        }
    };

    let total_pages = ((total + HISTORY_PAGE_SIZE - 1) / HISTORY_PAGE_SIZE).max(1);

    let logs = logs.into_iter()
        .map(RecentLogView::from)
        .collect();

    render(&HistoryTemplate {
        nav_active: "history",
        key_preview: api_key.client_name.clone(),
        rate_limit: api_key.rate_limit,
        method_selected,
        status_value,
        endpoint_value,
        date_from_value,
        date_to_value,
        logs,
        page,
        total_pages,
        total,
        has_prev: page > 1,
        has_next: page < total_pages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_seconds_defaults_to_one_hour() {
        assert_eq!(bucket_seconds("6h"), 6 * 3600);
        assert_eq!(bucket_seconds("24h"), 24 * 3600);
        assert_eq!(bucket_seconds("garbage"), 3600);
    }

    #[test]
    fn status_class_groups_expected_codes() {
        assert!(status_class(200).contains("text-win"));
        assert!(status_class(404).contains("text-brand"));
        assert!(status_class(429).contains("text-brand"));
        assert!(status_class(400).contains("text-loss"));
        assert!(status_class(500).contains("text-loss"));
    }

    #[test]
    fn sort_order_clause_rejects_unknown_values() {
        assert_eq!(sort_order_clause("status"), "ORDER BY status_code DESC, id DESC");
        assert_eq!(sort_order_clause("anything-else"), "ORDER BY id DESC");
    }

    #[test]
    fn client_ip_prefers_forwarded_header() {
        let fallback: SocketAddr = "10.0.0.1:1234".parse().unwrap();

        let mut headers = HeaderMap::new();
        headers.insert("x-forwarded-for", "203.0.113.5, 10.0.0.2".parse().unwrap());
        assert_eq!(client_ip(&headers, fallback), "203.0.113.5");

        assert_eq!(client_ip(&HeaderMap::new(), fallback), "10.0.0.1");
    }
}
