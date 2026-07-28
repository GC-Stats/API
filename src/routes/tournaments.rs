/*
    GC-Stats — API

    `/v1/tournaments` endpoints: search tournaments by name, fetch a
    tournament's full details with its phases, and its logo history.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use axum::{extract::{Path, State}, Json, http::StatusCode, Router};
use std::sync::Arc;
use axum::routing::get;
use crate::AppState;
use crate::models::tournament::{Tournament, TournamentFullResponse, TournamentPhase};
use crate::models::entity::{fetch_current_logo_ids, partition_logo_history, LogoUrls, LogoRow, LogoHistoryResponse};
use crate::util::escape_like;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/by-name/{name}", get(get_tournament_by_name))
        .route("/{id}", get(get_tournament))
        .route("/{id}/logos", get(get_tournament_logos))
}

#[utoipa::path(
    get,
    path = "/v1/tournaments/by-name/{name}",
    responses(
        (status = 200, description = "Tournament found", body = [TournamentFullResponse]),
        (status = 404, description = "Tournament not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Tournaments"
)]

pub async fn get_tournament_by_name(
    Path(name_query): Path<String>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<Vec<TournamentFullResponse>>, StatusCode> {

    let search_pattern = format!("{}%", escape_like(&name_query));

    let rows: Vec<_> = sqlx::query!(
        r#"
        SELECT
            t.id, t.name, t.region, t.category, t.description,
            t.prize_pool, t.location, t.start_date, t.end_date, t.status,
            tp.id as "phase_id?",
            tp.name as "phase_name?",
            tp.format as "phase_format?",
            tp.parent_id as "phase_parent?",
            GROUP_CONCAT(DISTINCT m.id) as "phase_match_ids?"
        FROM tournaments t
        LEFT JOIN tournament_phases tp ON t.id = tp.tournament_id
        LEFT JOIN matches m ON tp.id = m.phase_id
        WHERE t.name LIKE ?
        GROUP BY t.id, tp.id
        ORDER BY t.name, tp.id
        LIMIT 20
        "#,
        search_pattern
    )
        .fetch_all(&state.db_read)
        .await
        .map_err(|e| {
            tracing::error!("DB error on tournaments by-name: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    // Rows are ordered by tournament name, so grouping preserves a stable
    // response order (a HashMap here would shuffle results between calls).
    let mut tournaments: Vec<TournamentFullResponse> = Vec::new();

    for row in rows {
        let entry = match tournaments.iter_mut().find(|t| t.tournament.id == row.id) {
            Some(entry) => entry,
            None => {
                tournaments.push(TournamentFullResponse {
                    tournament: Tournament {
                        id: row.id,
                        name: row.name,
                        region: row.region,
                        category: row.category,
                        prize_pool: row.prize_pool,
                        location: row.location,
                        start_date: row.start_date,
                        end_date: row.end_date,
                        status: row.status,
                        description: row.description,
                    },
                    phases: Vec::new(),
                    logo: None,
                });
                tournaments.last_mut().unwrap()
            }
        };

        if let Some(p_id) = row.phase_id && !entry.phases.iter().any(|p| p.id == p_id) {
            let match_ids = row.phase_match_ids
                .map(|s| s.split(',').filter_map(|id_str| id_str.parse::<i64>().ok()).collect())
                .unwrap_or_default();

            entry.phases.push(TournamentPhase {
                id: p_id,
                tournament_id: entry.tournament.id,
                name: row.phase_name.unwrap_or_default(),
                format: Some(row.phase_format.unwrap_or_default()),
                parent_id: row.phase_parent,
                match_ids,
            });
        }
    }

    let tournament_ids: Vec<u64> = tournaments.iter().map(|t| t.tournament.id).collect();
    let mut logo_ids = fetch_current_logo_ids(&state.db_read, "tournament", &tournament_ids).await;

    for t in &mut tournaments {
        t.logo = logo_ids.remove(&t.tournament.id).map(|uuid| LogoUrls::build("tournaments", &uuid));
    }

    Ok(Json(tournaments))
}

#[utoipa::path(
    get,
    path = "/v1/tournaments/{id}",
    responses(
        (status = 200, description = "Tournament found", body = TournamentFullResponse),
        (status = 404, description = "Tournament not found"),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Tournaments"
)]

pub async fn get_tournament(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<TournamentFullResponse>, StatusCode> {
    let rows: Vec<_> = sqlx::query!(
        r#"
        SELECT
            t.id,
            t.name,
            t.region,
            t.category,
            t.description,
            t.prize_pool,
            t.location,
            t.start_date,
            t.end_date,
            t.status,
            tp.id as "phase_id?",
            tp.name as "phase_name?",
            tp.format as "phase_format?",
            tp.parent_id as "phase_parent?",
            GROUP_CONCAT(DISTINCT m.id) as "phase_match_ids?"
        FROM tournaments t
        LEFT JOIN tournament_phases tp ON t.id = tp.tournament_id
        LEFT JOIN matches m ON tp.id = m.phase_id
        WHERE t.id = ?
        GROUP BY t.id, tp.id
        "#,
        id
    )
        .fetch_all(&state.db_read)
        .await
        .map_err(|e| {
            tracing::error!("DB error on tournament by id: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    if rows.is_empty() {
        return Err(StatusCode::NOT_FOUND);
    }

    let first = &rows[0];
    let tournament = Tournament {
        id: first.id,
        name: first.name.clone(),
        region: first.region.clone(),
        category: first.category.clone(),
        prize_pool: first.prize_pool.clone(),
        location: first.location.clone(),
        start_date: first.start_date,
        end_date: first.end_date,
        status: first.status.clone(),
        description: first.description.clone(),
    };

    let phases: Vec<TournamentPhase> = rows
        .into_iter()
        .filter_map(|row| {
            row.phase_id.map(|p_id| {
                let match_ids = row.phase_match_ids
                    .map(|s| s.split(',').filter_map(|id| id.parse::<i64>().ok()).collect())
                    .unwrap_or_default();

                TournamentPhase {
                    id: p_id,
                    tournament_id: row.id,
                    name: row.phase_name.unwrap_or_default(),
                    format: Some(row.phase_format.unwrap_or_default()),
                    parent_id: row.phase_parent,
                    match_ids,
                }
            })
        })
        .collect();

    let logo = fetch_current_logo_ids(&state.db_read, "tournament", &[id])
        .await
        .remove(&id)
        .map(|uuid| LogoUrls::build("tournaments", &uuid));

    Ok(Json(TournamentFullResponse { tournament, phases, logo }))
}

#[utoipa::path(
    get,
    path = "/v1/tournaments/{id}/logos",
    responses(
        (status = 200, description = "Tournament logos", body = LogoHistoryResponse),
        (status = 401, description = "Unauthorized"),
        (status = 429, description = "Rate limited"),
    ),
    tag = "Tournaments"
)]

pub async fn get_tournament_logos(
    Path(id): Path<u64>,
    State(state): State<Arc<AppState>>,
) -> Result<Json<LogoHistoryResponse>, StatusCode> {
    let rows = sqlx::query_as::<_, LogoRow>(
        "SELECT id, `from`, until FROM logos WHERE entity_type = 'tournament' AND entity_id = ? ORDER BY `from` DESC"
    )
        .bind(id)
        .fetch_all(&state.db_read)
        .await
        .map_err(|e| {
            tracing::error!("DB error on tournament logos: {:?}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(partition_logo_history(rows, "tournaments")))
}
