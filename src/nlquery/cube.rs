/*
    GC-Stats — API

    Cube.dev REST client: fetches the `/meta` catalogue (measures/dimensions)
    and executes validated queries against `/load`. Two schema views are
    exposed:

    - get_cube_schema(): filtered down to the configured LLM-safe views only
      (raw cubes excluded — their fields can be ambiguous across join
      paths, e.g. two different notions of "kills" on unjoined cubes). Used
      by the nl-query pipeline, where the model can't be trusted to
      disambiguate that itself.
    - get_full_cube_schema(): every cube and view from `/meta`, unfiltered.
      Used by the query builder, where a human picks fields deliberately —
      Cube's own `/load` call still rejects an impossible join, surfaced as
      a normal cube_execution_failed error.

    Both cache their result in Redis (separate keys) with a TTL, since the
    catalogue doesn't change on every request.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use std::collections::HashSet;
use std::sync::Arc;

use redis::AsyncCommands;
use serde::{Deserialize, Serialize};

use crate::nlquery::error::NlQueryError;
use crate::nlquery::query::CubeQuery;
use crate::AppState;

const SCHEMA_CACHE_KEY: &str = "nlquery:cube_schema";
const FULL_SCHEMA_CACHE_KEY: &str = "nlquery:cube_full_schema";

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CubeSchemaSet {
    pub measures: HashSet<String>,
    pub dimensions: HashSet<String>,
}

#[derive(Deserialize)]
struct CubeMetaResponse {
    #[serde(default)]
    cubes: Vec<CubeMetaCube>,
}

#[derive(Deserialize)]
struct CubeMetaCube {
    name: String,
    #[serde(default)]
    measures: Vec<CubeMetaField>,
    #[serde(default)]
    dimensions: Vec<CubeMetaField>,
}

#[derive(Deserialize)]
struct CubeMetaField {
    name: String,
}

/// Keeps only the entries from `/meta` that belong to one of `view_names`,
/// discarding raw cubes. Raw cubes are excluded on purpose: the LLM should
/// only ever see fields from a disambiguated view (explicit join path,
/// usage-oriented description), never a raw cube's ambiguous measures.
fn cube_schema_from_meta(meta: &CubeMetaResponse, view_names: &[String]) -> CubeSchemaSet {
    let mut measures = HashSet::new();
    let mut dimensions = HashSet::new();

    for cube in &meta.cubes {
        if !view_names.iter().any(|v| v == &cube.name) {
            continue;
        }
        measures.extend(cube.measures.iter().map(|f| f.name.clone()));
        dimensions.extend(cube.dimensions.iter().map(|f| f.name.clone()));
    }

    CubeSchemaSet { measures, dimensions }
}

/// Keeps every cube and view from `/meta` — no filtering at all.
fn full_cube_schema_from_meta(meta: &CubeMetaResponse) -> CubeSchemaSet {
    let mut measures = HashSet::new();
    let mut dimensions = HashSet::new();

    for cube in &meta.cubes {
        measures.extend(cube.measures.iter().map(|f| f.name.clone()));
        dimensions.extend(cube.dimensions.iter().map(|f| f.name.clone()));
    }

    CubeSchemaSet { measures, dimensions }
}

async fn fetch_cube_meta(state: &Arc<AppState>) -> Result<CubeMetaResponse, NlQueryError> {
    let mut req = state
        .http_client
        .get(format!("{}/cubejs-api/v1/meta", state.nlquery.cube_api_url));
    if let Some(secret) = &state.nlquery.cube_api_secret {
        req = req.bearer_auth(secret);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| NlQueryError::CubeExecutionFailed(format!("Cube meta request failed: {e}")))?;

    if !resp.status().is_success() {
        return Err(NlQueryError::CubeExecutionFailed(format!(
            "Cube meta endpoint returned {}",
            resp.status()
        )));
    }

    resp.json()
        .await
        .map_err(|e| NlQueryError::CubeExecutionFailed(format!("invalid Cube meta response: {e}")))
}

async fn cached_schema<F>(
    state: &Arc<AppState>,
    cache_key: &str,
    build: F,
) -> Result<CubeSchemaSet, NlQueryError>
where
    F: FnOnce(&CubeMetaResponse) -> CubeSchemaSet,
{
    let mut redis = state.redis.clone();

    if let Ok(Some(json)) = redis.get::<_, Option<String>>(cache_key).await {
        if let Ok(schema) = serde_json::from_str::<CubeSchemaSet>(&json) {
            return Ok(schema);
        }
    }

    let meta = fetch_cube_meta(state).await?;
    let schema = build(&meta);

    if let Ok(json) = serde_json::to_string(&schema) {
        let _: Result<(), _> = redis
            .set_ex(cache_key, json, state.nlquery.cube_schema_cache_ttl_secs)
            .await;
    }

    Ok(schema)
}

/// The LLM-safe schema (configured views only). Not invalidated
/// automatically — the cached entry simply expires after
/// `cube_schema_cache_ttl_secs`.
pub async fn get_cube_schema(state: &Arc<AppState>) -> Result<CubeSchemaSet, NlQueryError> {
    cached_schema(state, SCHEMA_CACHE_KEY, |meta| {
        cube_schema_from_meta(meta, &state.nlquery.cube_views)
    })
    .await
}

/// The full, unfiltered schema (every cube and view) — feeds the query
/// builder's field pickers, where there's no LLM to mislead with an
/// ambiguous field.
pub async fn get_full_cube_schema(state: &Arc<AppState>) -> Result<CubeSchemaSet, NlQueryError> {
    cached_schema(state, FULL_SCHEMA_CACHE_KEY, full_cube_schema_from_meta).await
}

/// Executes an already-validated query against Cube's `/load` endpoint and
/// returns the `data` array.
pub async fn execute_cube_query(state: &Arc<AppState>, query: &CubeQuery) -> Result<serde_json::Value, NlQueryError> {
    let mut req = state
        .http_client
        .post(format!("{}/cubejs-api/v1/load", state.nlquery.cube_api_url))
        .json(&serde_json::json!({ "query": query }));
    if let Some(secret) = &state.nlquery.cube_api_secret {
        req = req.bearer_auth(secret);
    }

    let resp = req
        .send()
        .await
        .map_err(|e| NlQueryError::CubeExecutionFailed(format!("Cube load request failed: {e}")))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(NlQueryError::CubeExecutionFailed(format!("Cube returned {status}: {body}")));
    }

    let mut payload: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| NlQueryError::CubeExecutionFailed(format!("invalid Cube load response: {e}")))?;

    Ok(payload["data"].take())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mock_meta() -> CubeMetaResponse {
        serde_json::from_value(serde_json::json!({
            "cubes": [
                {
                    "name": "kill_stats",
                    "measures": [{ "name": "kill_stats.kills_count" }],
                    "dimensions": [{ "name": "kill_stats.player_name" }, { "name": "kill_stats.map_name" }]
                },
                {
                    "name": "game_map_round_player_stats",
                    "measures": [{ "name": "game_map_round_player_stats.kills" }],
                    "dimensions": [{ "name": "game_map_round_player_stats.player_id" }]
                },
                {
                    "name": "game_map_round_kills",
                    "measures": [{ "name": "game_map_round_kills.count" }],
                    "dimensions": []
                }
            ]
        }))
        .unwrap()
    }

    #[test]
    fn cube_schema_from_meta_keeps_only_configured_views() {
        let schema = cube_schema_from_meta(&mock_meta(), &["kill_stats".to_string()]);

        assert!(schema.measures.contains("kill_stats.kills_count"));
        assert!(schema.dimensions.contains("kill_stats.player_name"));
        assert!(schema.dimensions.contains("kill_stats.map_name"));

        assert!(!schema.measures.contains("game_map_round_player_stats.kills"));
        assert!(!schema.dimensions.contains("game_map_round_player_stats.player_id"));
        assert!(!schema.measures.contains("game_map_round_kills.count"));
    }

    #[test]
    fn cube_schema_from_meta_aggregates_multiple_views() {
        let schema = cube_schema_from_meta(
            &mock_meta(),
            &["kill_stats".to_string(), "game_map_round_kills".to_string()],
        );

        assert!(schema.measures.contains("kill_stats.kills_count"));
        assert!(schema.measures.contains("game_map_round_kills.count"));
        assert!(!schema.measures.contains("game_map_round_player_stats.kills"));
    }

    #[test]
    fn cube_schema_from_meta_empty_view_list_yields_empty_schema() {
        let schema = cube_schema_from_meta(&mock_meta(), &[]);

        assert!(schema.measures.is_empty());
        assert!(schema.dimensions.is_empty());
    }

    #[test]
    fn full_cube_schema_from_meta_keeps_every_cube() {
        let schema = full_cube_schema_from_meta(&mock_meta());

        assert!(schema.measures.contains("kill_stats.kills_count"));
        assert!(schema.measures.contains("game_map_round_player_stats.kills"));
        assert!(schema.measures.contains("game_map_round_kills.count"));
        assert!(schema.dimensions.contains("game_map_round_player_stats.player_id"));
    }
}
