/*
    GC-Stats — API

    Response models for maps and rounds: per-map player stats, map metadata,
    and per-round player stats.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use serde::{Serialize, Deserialize};
use sqlx::FromRow;
use utoipa::ToSchema;


#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct MapFullResponse {
    #[serde(flatten)]
    pub map: crate::models::game::GameMap,
    pub player_stats: Vec<crate::models::game::GamePlayerStat>,
    pub team_a: Option<crate::models::entity::TeamWithScore>,
    pub team_b: Option<crate::models::entity::TeamWithScore>,
}

#[derive(Debug, FromRow, Serialize, Deserialize, ToSchema)]
pub struct GamePlayerStat {
    pub id: u64,
    pub player_id: Option<u64>,
    pub team_id: u64,

    pub agent_name: String,

    pub kills: i32,
    pub assists: i32,
    pub deaths: i32,

    pub acs: i32,
    pub adr: i32,

    pub first_kills: i32,
    pub first_deaths: i32,
    pub kast_percentage: f64,
    pub headshot_percentage: f64,
}

#[derive(Debug, FromRow, Serialize, Deserialize, ToSchema)]
pub struct GameMap {
    pub id: u64,
    pub match_id: u64,

    pub api_match_id: Option<String>,
    pub map_name: String,

    pub team_a_score: i32,
    pub team_b_score: i32,

    pub order: i32,

    pub is_completed: bool,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RoundFullResponse {
    pub round_number: i32,
    pub winning_team: Option<u64>,
    pub win_type: Option<String>,
    pub player_stats: Vec<RoundPlayerStat>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct RoundPlayerStat {
    pub player_id: u64,
    pub kills: i32,
    pub assists: i32,
    pub score: i32,
    pub economy_spent: i32,
    pub economy_remaining: i32,
    pub weapon_id: Option<String>,
    pub armor: Option<String>,
}
