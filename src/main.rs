/*
    GC-Stats — API

    Application entrypoint. Loads configuration, connects to MariaDB and
    Redis, builds the Axum router (health check, versioned `/v1`
    API, Swagger UI) and starts the HTTP server.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

mod models;
mod routes;
mod middleware;
mod doc;
mod logging;
mod util;
mod nlquery;
mod llm;

use axum::{routing::get, Router};
use axum::middleware as ax_middleware;
use sqlx::mysql::MySqlPoolOptions;
use dotenvy::dotenv;
use std::env;
use axum::http::{header, HeaderValue, Method};
use utoipa_swagger_ui::{SwaggerUi};
use crate::doc::ApiDoc;
use utoipa::OpenApi;
use tower_http::trace::{TraceLayer, DefaultMakeSpan, DefaultOnResponse};
use tower_http::cors::{CorsLayer, Any};
use tower_http::normalize_path::NormalizePathLayer;
use tower_http::set_header::SetResponseHeaderLayer;
use tower::Layer;
use axum::ServiceExt;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};
use tracing::Level;

pub struct AppState {
    pub db_read: sqlx::MySqlPool,
    pub db_write: sqlx::MySqlPool,
    pub redis: redis::aio::ConnectionManager,
    pub redis_local: redis::aio::ConnectionManager,
    pub http_client: reqwest::Client,
    pub internal_api_secret: String,
    pub internal_auth_skip_timestamp_check: bool,
    pub nlquery: nlquery::config::NlQueryConfig,
}

async fn connect_pool(database_url: &str, max_connections: u32) -> sqlx::MySqlPool {
    MySqlPoolOptions::new()
        .max_connections(max_connections)
        .after_connect(|conn, _meta| Box::pin(async move {
            sqlx::query("SET SESSION group_concat_max_len = 1048576")
                .execute(&mut *conn)
                .await?;
            Ok(())
        }))
        .connect(database_url)
        .await
        .expect("Failed to connect to MariaDB")
}

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,tower_http=info".into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    dotenv().ok();

    rustls::crypto::ring::default_provider()
        .install_default()
        .expect("Failed to install rustls crypto provider");

    let database_url = std::env::var("DATABASE_URL").expect("DATABASE_URL missing");
    let max_connections = env::var("DATABASE_MAX_CONNECTIONS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(10);

    // Single-instance mode: DATABASE_URL serves both reads and writes.
    // Multi-AZ mode: set DATABASE_URL_WRITE to point writes at the primary
    // while DATABASE_URL targets a read replica.
    let db_read = connect_pool(&database_url, max_connections).await;
    let db_write = match env::var("DATABASE_URL_WRITE") {
        Ok(write_url) => connect_pool(&write_url, max_connections).await,
        Err(_) => db_read.clone(),
    };

    let redis_url = std::env::var("REDIS_URL").expect("REDIS_URL missing");
    let client = redis::Client::open(redis_url).expect("Invalid Redis URL");

    let redis_manager = client
        .get_connection_manager()
        .await
        .expect("Failed to connect to Redis");

    let redis_local_url = std::env::var("REDIS_LOCAL_URL").expect("REDIS_LOCAL_URL missing");
    let redis_local_client = redis::Client::open(redis_local_url).expect("Invalid Redis local URL");

    let redis_local_manager = redis_local_client
        .get_connection_manager()
        .await
        .expect("Failed to connect to local Redis");

    let http_client = reqwest::Client::builder()
        .build()
        .expect("Failed to build HTTP client");

    let internal_api_secret = std::env::var("INTERNAL_API_SECRET").expect("INTERNAL_API_SECRET missing");

    // TODO(temp/testing): remove once Laravel-side testing is done — skips
    // the replay-protection skew check entirely, do not leave enabled in production.
    let internal_auth_skip_timestamp_check = env::var("INTERNAL_AUTH_SKIP_TIMESTAMP_CHECK")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    let nlquery_config = nlquery::config::NlQueryConfig {
        cube_api_url: std::env::var("CUBE_API_URL").expect("CUBE_API_URL missing"),
        cube_api_secret: env::var("CUBE_API_SECRET").ok(),
        cube_schema_cache_ttl_secs: env::var("CUBE_SCHEMA_CACHE_TTL_SECS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(3600),
        cube_views: env::var("CUBE_VIEW_NAMES")
            .unwrap_or_else(|_| {
                "kill_stats,player_stats,player_advanced_stats,damage_stats,round_results,\
                 map_results,match_results,round_state_stats,player_positions,tournament_info,\
                 tournament_phases_info,tournament_participants,roster,match_schedule,\
                 match_vetos_info,point_entries_info"
                    .to_string()
            })
            .split(',')
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .collect(),
        platform_llm_provider: env::var("PLATFORM_LLM_PROVIDER").unwrap_or_else(|_| "openai".to_string()),
        platform_llm_api_key: env::var("PLATFORM_LLM_API_KEY").unwrap_or_default(),
        openai_model: env::var("OPENAI_MODEL").unwrap_or_else(|_| "gpt-4o-mini".to_string()),
        anthropic_model: env::var("ANTHROPIC_MODEL").unwrap_or_else(|_| "claude-3-5-haiku-20241022".to_string()),
        timeout_ms: env::var("NL_QUERY_TIMEOUT_MS").ok().and_then(|v| v.parse().ok()).unwrap_or(12_000),
    };

    let shared_state = std::sync::Arc::new(AppState {
        db_read,
        db_write,
        redis: redis_manager,
        redis_local: redis_local_manager,
        http_client,
        internal_api_secret,
        internal_auth_skip_timestamp_check,
        nlquery: nlquery_config,
    });

    tokio::spawn(logging::flusher::run(shared_state.clone()));

    let app_env = env::var("APP_ENV").unwrap_or_else(|_| "development".to_string());

    let cors = if app_env == "production" {
        let origin = env::var("ALLOWED_ORIGIN")
            .expect("ALLOWED_ORIGIN must be set in production");

        CorsLayer::new()
            .allow_origin(origin.parse::<header::HeaderValue>().unwrap())
            .allow_methods([Method::GET])
            .allow_headers(Any)
    } else {
        CorsLayer::new()
            .allow_origin(Any)
            .allow_methods(Any)
            .allow_headers(Any)
    };

    let inner = Router::new()
        .route("/health", get(routes::health::health_check))

        .nest("/v1", routes::api_router_v1().layer(
            ax_middleware::from_fn_with_state(
                shared_state.clone(),
                middleware::auth::mw_rate_limiter
            )
        ).layer(
            ax_middleware::from_fn_with_state(
                shared_state.clone(),
                middleware::logging::mw_request_logger
            )
        ))

        .nest("/v2", routes::api_router_v2().layer(
            ax_middleware::from_fn_with_state(
                shared_state.clone(),
                middleware::auth::mw_rate_limiter
            )
        ).layer(
            ax_middleware::from_fn_with_state(
                shared_state.clone(),
                middleware::logging::mw_request_logger
            )
        ))

        .nest("/internal", routes::internal::router().layer(
            ax_middleware::from_fn_with_state(
                shared_state.clone(),
                middleware::auth::mw_internal_auth
            )
        ))

        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(Level::INFO))
                .on_response(DefaultOnResponse::new().level(Level::INFO))
        )
        .layer(cors)
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::X_FRAME_OPTIONS,
            HeaderValue::from_static("DENY"),
        ))
        .layer(SetResponseHeaderLayer::if_not_present(
            header::REFERRER_POLICY,
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ))

        .with_state(shared_state);

    // SwaggerUi issues its own redirect from "/doc" to "/doc/", which loops forever
    // if NormalizePathLayer::trim_trailing_slash also strips "/doc/" back to "/doc".
    // Keep it outside the normalized subtree so its own routing owns the trailing slash.
    let inner = NormalizePathLayer::trim_trailing_slash().layer(inner);

    let app = Router::new()
        .merge(SwaggerUi::new("/doc").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .fallback_service(inner);
    let app = ServiceExt::<axum::extract::Request>::into_make_service_with_connect_info::<std::net::SocketAddr>(app);

    let bind_addr = env::var("BIND_ADDR").unwrap_or_else(|_| "0.0.0.0:3000".to_string());
    let listener = tokio::net::TcpListener::bind(&bind_addr).await
        .unwrap_or_else(|err| panic!("Failed to bind {}: {}", bind_addr, err));
    axum::serve(listener, app).await.unwrap();
}