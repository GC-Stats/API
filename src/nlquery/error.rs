/*
    GC-Stats — API

    Error type for the nl-query pipeline. Unlike the rest of the API (which
    returns bare status codes), Laravel needs to tell these failure modes
    apart to show a relevant message, so each variant carries its own status
    code and a stable `error` code in a small JSON body.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use axum::{http::StatusCode, response::{IntoResponse, Response}, Json};

#[derive(Debug)]
pub enum NlQueryError {
    /// The LLM call itself failed (network error, non-2xx, missing/malformed
    /// tool-call envelope) — not a problem with the query it produced.
    LlmCallFailed(String),
    /// The model returned something that isn't a valid Cube query: either
    /// not parseable JSON, or a measure/dimension/filter member that doesn't
    /// exist in the real Cube schema.
    InvalidCubeQuery(String),
    /// The query was valid but Cube failed to execute it.
    CubeExecutionFailed(String),
    /// The total time budget (LLM + Cube) was exceeded.
    Timeout,
    /// The server's own configuration is invalid (e.g. an unrecognized
    /// `platform_llm_provider` value) — not something the caller can fix.
    Config(String),
}

impl IntoResponse for NlQueryError {
    fn into_response(self) -> Response {
        let (status, code, message) = match self {
            NlQueryError::LlmCallFailed(message) => (StatusCode::BAD_GATEWAY, "llm_call_failed", message),
            NlQueryError::InvalidCubeQuery(message) => (StatusCode::UNPROCESSABLE_ENTITY, "invalid_cube_query", message),
            NlQueryError::CubeExecutionFailed(message) => (StatusCode::BAD_GATEWAY, "cube_execution_failed", message),
            NlQueryError::Timeout => (StatusCode::GATEWAY_TIMEOUT, "timeout", "The request exceeded the allotted time budget".to_string()),
            NlQueryError::Config(message) => (StatusCode::INTERNAL_SERVER_ERROR, "config_error", message),
        };

        (status, Json(serde_json::json!({ "error": code, "message": message }))).into_response()
    }
}
