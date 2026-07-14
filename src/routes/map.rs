/*
    GC-Stats — API

    `/v1/map` endpoints: fetch a map's full details (teams, player stats)
    and its per-round breakdown.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use std::collections::{BTreeMap, HashSet};
use axum::{extract::{Path, State}, Json, http::StatusCode, Router};
use std::sync::Arc;
use axum::routing::get;
use crate::AppState;
use crate::models::entity::{Team, TeamWithScore};
use crate::models::game::{MapFullResponse, RoundFullResponse, RoundPlayerStat};

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/{id}", get(get_map))
        .route("/{id}/rounds", get(get_map_rounds))
}

#[utoipa::path(
    get,
    path = "/v1/map/{id}",
    responses(
        (status = 200, description = "Map found", body = MapFullResponse),
        (status = 404, description = "Map not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Maps"
)]

pub async fn get_map(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<MapFullResponse>, StatusCode> {

    let rows: Vec<_> = sqlx::query!(
        r#"
        SELECT
            m.id, m.match_id, m.api_match_id, m.map_name,
            m.team_a_score as "team_a_score!", m.team_b_score as "team_b_score!", m.order as map_order, m.is_completed,

            -- Team A
            ta.id as "ta_id?", ta.name as "ta_name?", ta.short_name as "ta_short_name?",
            ta.country_code as "ta_country_code?", ta.bio as "ta_bio?",
            ta.socials as "ta_socials?", ta.vlr_id as "ta_vlr_id?", ta.is_active as "ta_is_active?",

            -- Team B
            tb.id as "tb_id?", tb.name as "tb_name?", tb.short_name as "tb_short_name?",
            tb.country_code as "tb_country_code?", tb.bio as "tb_bio?",
            tb.socials as "tb_socials?", tb.vlr_id as "tb_vlr_id?", tb.is_active as "tb_is_active?",

            -- Game Player Stats
            gps.id as "player_stats_id?",
            gps.player_id as "player_id?", gps.team_id as "team_id?",
            gps.agent_name as "agent_name?", gps.kills as "player_kills?",
            gps.deaths as "player_deaths?", gps.assists as "player_assists?",
            gps.acs as "player_acs?", gps.adr as "player_adr?",
            gps.first_kills as "player_first_kills?", gps.first_deaths as "player_first_deaths?",

            CAST(gps.kast_percentage AS DOUBLE) as "player_kast?",
            CAST(gps.headshot_percentage AS DOUBLE) as "player_hs?"


        FROM game_maps m
        LEFT JOIN matches ma ON m.match_id = ma.id
        LEFT JOIN teams ta ON ma.team_a_id = ta.id
        LEFT JOIN teams tb ON ma.team_b_id = tb.id
        LEFT JOIN game_player_stats gps ON m.id = gps.game_map_id
        WHERE m.id = ?
        ORDER BY gps.id ASC
        "#,
        id
    )
        .fetch_all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("DB error on map by id: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if rows.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let first = &rows[0];

    let map = crate::models::game::GameMap {
        id: first.id,
        match_id: first.match_id,
        api_match_id: first.api_match_id.clone(),
        map_name: first.map_name.clone(),
        team_a_score: first.team_a_score,
        team_b_score: first.team_b_score,
        order: first.map_order,
        is_completed: first.is_completed != 0,
    };

    let team_a = first.ta_id.map(|team_id| TeamWithScore {
        team: Team::from_joined_row(
            team_id,
            first.ta_name.clone(),
            first.ta_short_name.clone(),
            first.ta_country_code.clone(),
            first.ta_socials.as_deref(),
            first.ta_bio.clone(),
            first.ta_vlr_id,
            first.ta_is_active,
        ),
        score: first.team_a_score,
    });

    let team_b = first.tb_id.map(|team_id| TeamWithScore {
        team: Team::from_joined_row(
            team_id,
            first.tb_name.clone(),
            first.tb_short_name.clone(),
            first.tb_country_code.clone(),
            first.tb_socials.as_deref(),
            first.tb_bio.clone(),
            first.tb_vlr_id,
            first.tb_is_active,
        ),
        score: first.team_b_score,
    });

    let mut seen_player_stats = HashSet::new();
    let player_stats: Vec<crate::models::game::GamePlayerStat> = rows
        .iter()
        .filter_map(|r| {
            let stat_id = r.player_stats_id?;
            if seen_player_stats.insert(stat_id) {
                Some(crate::models::game::GamePlayerStat {
                    id: stat_id,
                    player_id: Some(r.player_id?),
                    team_id: r.team_id?,
                    agent_name: r.agent_name.clone().unwrap_or_default(),
                    kills: r.player_kills.unwrap_or(0),
                    deaths: r.player_deaths.unwrap_or(0),
                    assists: r.player_assists.unwrap_or(0),
                    acs: r.player_acs.unwrap_or(0),
                    adr: r.player_adr.unwrap_or(0),
                    first_kills: r.player_first_kills.unwrap_or(0),
                    first_deaths: r.player_first_deaths.unwrap_or(0),
                    kast_percentage: r.player_kast.unwrap_or(0.0),
                    headshot_percentage: r.player_hs.unwrap_or(0.0),
                })
            } else {
                None
            }
        })
        .collect();

    Ok(Json(MapFullResponse {
        map,
        player_stats,
        team_a,
        team_b,
    }))
}

#[utoipa::path(
    get,
    path = "/v1/map/{id}/rounds",
    responses(
        (status = 200, description = "Map found", body = RoundFullResponse),
        (status = 404, description = "Map not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Maps"
)]

pub async fn get_map_rounds(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<RoundFullResponse>>, StatusCode> {

    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM game_maps WHERE id = ?)")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }

    let rows: Vec<_> = sqlx::query!(
        r#"
        SELECT
            r.id as round_id, r.round_number, r.winning_team, r.win_type,
            ps.player_id as "player_id?", ps.kills as "ps_kills?",
            ps.assists as "ps_assists?", ps.score as "ps_score?",
            ps.economy_spent, ps.economy_remaining, ps.weapon_id, ps.armor
        FROM game_map_rounds r
        LEFT JOIN game_map_round_player_stats ps ON r.id = ps.game_map_round_id
        WHERE r.game_map_id = ?
        ORDER BY r.round_number ASC
        "#,
        id
    )
        .fetch_all(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut rounds_map: BTreeMap<i32, RoundFullResponse> = BTreeMap::new();

    for row in rows {
        let entry = rounds_map.entry(row.round_number).or_insert(RoundFullResponse {
            round_number: row.round_number,
            winning_team: Some(row.winning_team),
            win_type: Option::from(row.win_type),
            player_stats: Vec::new(),
        });

        if let Some(p_id) = row.player_id {
            entry.player_stats.push(RoundPlayerStat {
                player_id: p_id,
                kills: row.ps_kills.unwrap_or(0),
                assists: row.ps_assists.unwrap_or(0),
                score: row.ps_score.unwrap_or(0),
                economy_spent: row.economy_spent.unwrap_or(0),
                economy_remaining: row.economy_remaining.unwrap_or(0),
                weapon_id: row.weapon_id,
                armor: row.armor,
            });
        }
    }

    let result: Vec<RoundFullResponse> = rounds_map.into_values().collect();

    Ok(Json(result))
}


