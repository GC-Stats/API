/*
    GC-Stats — API

    `/v1/teams` endpoints: search teams by name, fetch a team's full
    profile, its current/past roster, and its logo history.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use axum::{extract::{Path, Query, State}, Json, http::StatusCode, Router};
use std::sync::Arc;
use axum::routing::get;
use crate::AppState;
use crate::models::entity::{fetch_current_logo_ids, parse_socials, partition_logo_history, Team, TeamPlayersResponse, TeamResponse, LogoUrls, LogoRow, LogoHistoryResponse};
use crate::models::stats::{fetch_team_maps, fetch_team_stats, fetch_weapon_stats, EntityKind, MapsQuery, StatsQuery, TeamMapEntry, TeamStatsResponse, WeaponStatsEntry};
use crate::util::escape_like;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/by-name/{name}", get(get_team_by_name))
        .route("/{id}", get(get_team))
        .route("/{id}/players", get(get_team_players))
        .route("/{id}/logos", get(get_team_logos))
        .route("/{id}/stats", get(get_team_stats))
        .route("/{id}/maps", get(get_team_maps))
        .route("/{id}/weapons", get(get_team_weapons))
}

#[utoipa::path(
    get,
    path = "/v1/teams/by-name/{name}",
    responses(
        (status = 200, description = "Team found", body = [TeamResponse]),
        (status = 404, description = "Team not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Teams"
)]

pub async fn get_team_by_name(
    Path(name_query): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<TeamResponse>>, StatusCode> {

    let search_pattern = format!("{}%", escape_like(&name_query));

    let teams = sqlx::query_as::<_, Team>(
        r#"
        SELECT id, name, short_name, country_code, socials, bio, vlr_id, is_active
        FROM teams
        WHERE name LIKE ? OR short_name LIKE ?
        LIMIT 10
        "#
    )
        .bind(&search_pattern)
        .bind(&search_pattern)
        .fetch_all(&state.db_read)
        .await
        .map_err(|e| {
            tracing::error!("DB error on teams by-name: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let team_ids: Vec<u64> = teams.iter().map(|t| t.id).collect();
    let mut logo_ids = fetch_current_logo_ids(&state.db_read, "team", &team_ids).await;

    let responses = teams.into_iter()
        .map(|team| {
            let logo = logo_ids.remove(&team.id).map(|uuid| LogoUrls::build("teams", &uuid));
            TeamResponse { team, logo }
        })
        .collect();

    Ok(Json(responses))
}

#[utoipa::path(
    get,
    path = "/v1/teams/{id}",
    responses(
        (status = 200, description = "Team found", body = TeamResponse),
        (status = 404, description = "Team not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Teams"
)]

pub async fn get_team(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<TeamResponse>, StatusCode> {

    let team = sqlx::query_as::<_, Team>(
        "SELECT id, name, short_name, country_code, socials, bio, vlr_id, is_active
         FROM teams
         WHERE id = ?"
    )
        .bind(id)
        .fetch_optional(&state.db_read)
        .await
        .map_err(|e| {
            tracing::error!("DB error on team by id: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let logo = fetch_current_logo_ids(&state.db_read, "team", &[id])
        .await
        .remove(&id)
        .map(|uuid| LogoUrls::build("teams", &uuid));

    Ok(Json(TeamResponse { team, logo }))
}

#[utoipa::path(
    get,
    path = "/v1/teams/{id}/players",
    responses(
        (status = 200, description = "Team found", body = [TeamPlayersResponse]),
        (status = 404, description = "Team not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Teams"
)]

pub async fn get_team_players(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<TeamPlayersResponse>, StatusCode> {
    let all_members: Vec<_> = sqlx::query!(
        r#"
        SELECT p.*, pt.left_at
        FROM players p
        JOIN player_team pt ON p.id = pt.player_id
        WHERE pt.team_id = ?
        "#,
        id
    )
        .fetch_all(&state.db_read)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let mut current = Vec::new();
    let mut history = Vec::new();

    for row in all_members {
        let player = crate::models::entity::Player {
            id: row.id,
            handle: row.handle,
            first_name: row.first_name,
            last_name: row.last_name,
            country_code: row.country_code,
            bio: row.bio,
            socials: parse_socials(&row.socials),
            vlr_id: row.vlr_id,
            is_active: row.is_active != 0,
        };

        if row.left_at.is_none() {
            current.push(player);
        } else {
            history.push(player);
        }
    }

    Ok(Json(TeamPlayersResponse { current, history }))
}

#[utoipa::path(
    get,
    path = "/v1/teams/{id}/logos",
    responses(
        (status = 200, description = "Team logos", body = LogoHistoryResponse),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Teams"
)]

pub async fn get_team_logos(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<LogoHistoryResponse>, StatusCode> {
    let rows = sqlx::query_as::<_, LogoRow>(
        "SELECT id, `from`, until FROM logos WHERE entity_type = 'team' AND entity_id = ? ORDER BY `from` DESC"
    )
        .bind(id)
        .fetch_all(&state.db_read)
        .await
        .map_err(|e| {
            tracing::error!("DB error on team logos: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(partition_logo_history(rows, "teams")))
}

#[utoipa::path(
    get,
    path = "/v1/teams/{id}/stats",
    params(StatsQuery),
    responses(
        (status = 200, description = "Average stats for the team, always broken down by side (atk/def), plus XvY situations and post-plant performance", body = TeamStatsResponse),
        (status = 404, description = "Team not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Teams"
)]

pub async fn get_team_stats(
    Path(id): Path<u64>,
    Query(filters): Query<StatsQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<TeamStatsResponse>, StatusCode> {
    let response = fetch_team_stats(&state.db_read, id, &filters)
        .await
        .map_err(|e| {
            tracing::error!("DB error on team stats: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/v1/teams/{id}/maps",
    params(MapsQuery),
    responses(
        (status = 200, description = "Maps played by the team (winrate + atk/def winrate), with the comps played on each map nested inside (aggregated across all tournaments unless `tournament_id` is passed)", body = [TeamMapEntry]),
        (status = 404, description = "Team not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Teams"
)]

pub async fn get_team_maps(
    Path(id): Path<u64>,
    Query(query): Query<MapsQuery>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<TeamMapEntry>>, StatusCode> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM teams WHERE id = ?)")
        .bind(id)
        .fetch_one(&state.db_read)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }

    let maps = fetch_team_maps(&state.db_read, id, query.tournament_id)
        .await
        .map_err(|e| {
            tracing::error!("DB error on team maps: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(maps))
}

#[utoipa::path(
    get,
    path = "/v1/teams/{id}/weapons",
    responses(
        (status = 200, description = "Weapon usage for the team: rounds held (when known) and kills landed", body = [WeaponStatsEntry]),
        (status = 404, description = "Team not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Teams"
)]

pub async fn get_team_weapons(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<WeaponStatsEntry>>, StatusCode> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM teams WHERE id = ?)")
        .bind(id)
        .fetch_one(&state.db_read)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }

    let weapons = fetch_weapon_stats(&state.db_read, EntityKind::Team, id)
        .await
        .map_err(|e| {
            tracing::error!("DB error on team weapons: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(weapons))
}
