/*
    GC-Stats — API

    `/v1/matches` endpoint: fetch a match's full details (teams, maps,
    vetoes).

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use axum::{extract::{Path, State}, Json, http::StatusCode, Router};
use std::sync::Arc;
use axum::routing::get;
use crate::AppState;
use crate::models::entity::{Team, TeamWithScore};
use crate::models::matchs::MatchFullResponse;
use crate::models::stats::{fetch_match_stats, MatchStatsResponse};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/{id}", get(get_match))
}

pub fn router_v2() -> Router<Arc<AppState>> {
    Router::new()
        .route("/{id}", get(get_match_v2))
}

#[utoipa::path(
    get,
    path = "/v1/matches/{id}",
    responses(
        (status = 200, description = "Match found", body = MatchFullResponse),
        (status = 404, description = "Match not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Matches"
)]

pub async fn get_match(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<MatchFullResponse>, StatusCode> {
    // Three independent queries instead of a single LEFT JOIN on both
    // game_maps and match_vetos, which produced a maps × vetos cartesian
    // product and did not guarantee map ordering.
    let match_query = sqlx::query!(
        r#"
        SELECT
            m.id, m.tournament_id, m.phase_id, m.round_number, m.scheduled_at,
            m.round_name, m.status, m.best_of, m.patch,
            m.team_a_score, m.team_b_score,

            ta.id as "ta_id?", ta.name as "ta_name?", ta.short_name as "ta_short_name?",
            ta.country_code as "ta_country_code?", ta.bio as "ta_bio?",
            ta.socials as "ta_socials?", ta.vlr_id as "ta_vlr_id?", ta.is_active as "ta_is_active?",

            tb.id as "tb_id?", tb.name as "tb_name?", tb.short_name as "tb_short_name?",
            tb.country_code as "tb_country_code?", tb.bio as "tb_bio?",
            tb.socials as "tb_socials?", tb.vlr_id as "tb_vlr_id?", tb.is_active as "tb_is_active?"
        FROM matches m
        LEFT JOIN teams ta ON m.team_a_id = ta.id
        LEFT JOIN teams tb ON m.team_b_id = tb.id
        WHERE m.id = ?
        "#,
        id
    )
        .fetch_optional(&state.db_read);

    let maps_query = sqlx::query!(
        r#"
        SELECT id, match_id, api_match_id, map_name,
               team_a_score as "team_a_score?", team_b_score as "team_b_score?", `order`, is_completed
        FROM game_maps
        WHERE match_id = ?
        ORDER BY `order` ASC
        "#,
        id
    )
        .fetch_all(&state.db_read);

    let vetos_query = sqlx::query!(
        r#"
        SELECT team_id, map_name, `type` as veto_type, `order`
        FROM match_vetos
        WHERE match_id = ?
        ORDER BY `order` ASC
        "#,
        id
    )
        .fetch_all(&state.db_read);

    let (match_row, map_rows, veto_rows) = tokio::join!(match_query, maps_query, vetos_query);

    let db_error = |e: sqlx::Error| {
        tracing::error!("DB error on match by id: {:?}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    };

    let row = match_row.map_err(db_error)?.ok_or(StatusCode::NOT_FOUND)?;
    let map_rows = map_rows.map_err(db_error)?;
    let veto_rows = veto_rows.map_err(db_error)?;

    let match_info = crate::models::matchs::Match {
        id: row.id,
        tournament_id: row.tournament_id,
        phase_id: row.phase_id,
        round_name: row.round_name,
        round_number: row.round_number,
        scheduled_at: row.scheduled_at,
        status: row.status,
        best_of: row.best_of,
        patch: row.patch,
    };

    let team_a = row.ta_id.map(|team_id| TeamWithScore {
        team: Team::from_joined_row(
            team_id,
            row.ta_name,
            row.ta_short_name,
            row.ta_country_code,
            row.ta_socials.as_deref(),
            row.ta_bio,
            row.ta_vlr_id,
            row.ta_is_active,
        ),
        score: Some(row.team_a_score),
    });

    let team_b = row.tb_id.map(|team_id| TeamWithScore {
        team: Team::from_joined_row(
            team_id,
            row.tb_name,
            row.tb_short_name,
            row.tb_country_code,
            row.tb_socials.as_deref(),
            row.tb_bio,
            row.tb_vlr_id,
            row.tb_is_active,
        ),
        score: Some(row.team_b_score),
    });

    let maps = map_rows.into_iter()
        .map(|r| crate::models::game::GameMap {
            id: r.id,
            match_id: r.match_id,
            api_match_id: r.api_match_id,
            map_name: r.map_name,
            team_a_score: r.team_a_score,
            team_b_score: r.team_b_score,
            order: r.order,
            is_completed: r.is_completed != 0,
        })
        .collect();

    let vetos = veto_rows.into_iter()
        .map(|r| crate::models::matchs::MatchVeto {
            match_id: match_info.id,
            team_id: r.team_id,
            map_name: r.map_name,
            r#type: r.veto_type,
            order: r.order,
        })
        .collect();

    Ok(Json(MatchFullResponse {
        match_info,
        maps,
        team_a,
        team_b,
        vetos,
    }))
}

#[utoipa::path(
    get,
    path = "/v2/matches/{id}",
    responses(
        (status = 200, description = "Match found, with global + full round-by-round stats for every map", body = MatchStatsResponse),
        (status = 404, description = "Match not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Matches"
)]

pub async fn get_match_v2(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<MatchStatsResponse>, StatusCode> {
    let response = fetch_match_stats(&state.db_read, id)
        .await
        .map_err(|e| {
            tracing::error!("DB error on match v2: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(response))
}
