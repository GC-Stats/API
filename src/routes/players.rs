/*
    GC-Stats — API

    `/v1/players` endpoints: search players by name, fetch a player's full
    profile, their team history, and their photo history.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use axum::{extract::{Path, State}, Json, http::StatusCode, Router};
use std::sync::Arc;
use axum::routing::get;
use crate::AppState;
use crate::models::entity::{fetch_current_logo_ids, parse_socials, partition_logo_history, Player, PlayerFullResponse, PlayerTeamHistory, Team, LogoUrls, LogoRow, LogoHistoryResponse};
use crate::util::escape_like;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/by-name/{name}", get(get_player_by_name))
        .route("/{id}", get(get_player))
        .route("/{id}/teams", get(get_player_teams))
        .route("/{id}/photos", get(get_player_photos))
}

#[utoipa::path(
    get,
    path = "/v1/players/by-name/{name}",
    responses(
        (status = 200, description = "Player found", body = [PlayerFullResponse]),
        (status = 404, description = "Player not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Players"
)]

pub async fn get_player_by_name(
    Path(name_query): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PlayerFullResponse>>, StatusCode> {

    let search_pattern = format!("{}%", escape_like(&name_query));

    let rows: Vec<_> = sqlx::query!(
        r#"
        SELECT
            p.id, p.handle, p.first_name, p.last_name,
            p.country_code, p.bio, p.socials, p.vlr_id, p.is_active,
            t.id as "team_id?", t.name as "team_name?", t.short_name as "team_short?",
            t.country_code as "team_country?", t.socials as "team_socials?",
            t.bio as "team_bio?", t.vlr_id as "team_vlr?", t.is_active as "team_active?"
        FROM players p
        LEFT JOIN player_team pt ON p.id = pt.player_id AND pt.left_at IS NULL
        LEFT JOIN teams t ON pt.team_id = t.id
        WHERE p.handle LIKE ?
        LIMIT 20
        "#,
        search_pattern
    )
        .fetch_all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("DB error on players by-name: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let player_ids: Vec<u64> = rows.iter().map(|r| r.id).collect();
    let mut photo_ids = fetch_current_logo_ids(&state.db, "player", &player_ids).await;

    let players = rows.into_iter()
        .map(|row| {
            let player = Player {
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

            let current_team = row.team_id.map(|t_id| Team::from_joined_row(
                t_id,
                row.team_name,
                row.team_short,
                row.team_country,
                row.team_socials.as_deref(),
                row.team_bio,
                row.team_vlr,
                row.team_active,
            ));

            let photo = photo_ids.remove(&player.id).map(|uuid| LogoUrls::build("players", &uuid));

            PlayerFullResponse { player, current_team, photo }
        })
        .collect();

    Ok(Json(players))
}

#[utoipa::path(
    get,
    path = "/v1/players/{id}",
    responses(
        (status = 200, description = "Player found", body = PlayerFullResponse),
        (status = 404, description = "Player not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Players"
)]

pub async fn get_player(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<PlayerFullResponse>, StatusCode> {
    let row = sqlx::query!(
        r#"
        SELECT
            p.id as id, p.handle as handle, p.first_name as first_name, p.last_name as last_name,
            p.country_code as country_code, p.bio as bio, p.socials as socials, p.vlr_id as vlr_id, p.is_active as is_active,
            t.id as team_id, t.name as team_name, t.short_name as team_short,
            t.country_code as team_country, t.socials as team_socials,
            t.bio as team_bio, t.vlr_id as team_vlr, t.is_active as team_active
        FROM players p
        LEFT JOIN player_team pt ON p.id = pt.player_id AND pt.left_at IS NULL
        LEFT JOIN teams t ON pt.team_id = t.id
        WHERE p.id = ?
        "#,
        id
    )
        .fetch_optional(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("DB error on player by id: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
        .ok_or(StatusCode::NOT_FOUND)?;

    let player = Player {
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

    let current_team = row.team_id.map(|t_id| Team::from_joined_row(
        t_id,
        row.team_name,
        row.team_short,
        row.team_country,
        row.team_socials.as_deref(),
        row.team_bio,
        row.team_vlr,
        row.team_active,
    ));

    let photo = fetch_current_logo_ids(&state.db, "player", &[id])
        .await
        .remove(&id)
        .map(|uuid| LogoUrls::build("players", &uuid));

    Ok(Json(PlayerFullResponse { player, current_team, photo }))
}

#[utoipa::path(
    get,
    path = "/v1/players/{id}/teams",
    responses(
        (status = 200, description = "Player found", body = [PlayerTeamHistory]),
        (status = 404, description = "Player not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Players"
)]

pub async fn get_player_teams(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<PlayerTeamHistory>>, StatusCode> {

    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM players WHERE id = ?)")
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
            t.id as team_id,
            t.name as team_name,
            t.short_name as team_short,
            t.country_code as team_country,
            pt.role,
            pt.joined_at,
            pt.left_at
        FROM player_team pt
        JOIN teams t ON pt.team_id = t.id
        WHERE pt.player_id = ?
        ORDER BY pt.joined_at DESC
        "#,
        id
    )
        .fetch_all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("DB error on player team history: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    let history = rows.into_iter().map(|row| PlayerTeamHistory {
        team_id: row.team_id,
        team_name: row.team_name,
        team_short_name: row.team_short,
        team_country: row.team_country,
        role: row.role,
        joined_at: row.joined_at,
        left_at: row.left_at,
    }).collect();

    Ok(Json(history))
}

#[utoipa::path(
    get,
    path = "/v1/players/{id}/photos",
    responses(
        (status = 200, description = "Player photos", body = LogoHistoryResponse),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Players"
)]

pub async fn get_player_photos(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<LogoHistoryResponse>, StatusCode> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM players WHERE id = ?)")
        .bind(id)
        .fetch_one(&state.db)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    if !exists {
        return Err(StatusCode::NOT_FOUND);
    }

    let rows = sqlx::query_as::<_, LogoRow>(
        "SELECT id, `from`, until FROM logos WHERE entity_type = 'player' AND entity_id = ? ORDER BY `from` DESC"
    )
        .bind(id)
        .fetch_all(&state.db)
        .await
        .map_err(|e| {
            tracing::error!("DB error on player photos: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(partition_logo_history(rows, "players")))
}
