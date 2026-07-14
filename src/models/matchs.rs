/*
    GC-Stats — API

    Response models for matches: the match record itself and its map
    vetoes.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use serde::{Serialize, Deserialize};
use sqlx::FromRow;
use chrono::{ NaiveDateTime };
use utoipa::ToSchema;

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct MatchFullResponse {
    #[serde(flatten)]
    pub match_info: crate::models::matchs::Match,
    pub maps: Vec<crate::models::game::GameMap>,
    pub team_a: Option<crate::models::entity::TeamWithScore>,
    pub team_b: Option<crate::models::entity::TeamWithScore>,
    pub vetos: Vec<crate::models::matchs::MatchVeto>,
}


#[derive(Debug, FromRow, Serialize, Deserialize,  ToSchema)]
pub struct Match {
    pub id: u64,
    pub tournament_id: u64,
    pub phase_id: u64,

    pub round_number: i32,
    pub round_name: String,

    pub scheduled_at: NaiveDateTime,

    pub status: String,

    pub best_of: i32,

    pub patch: Option<String>,
}

#[derive(Debug, FromRow, Serialize, Deserialize,  ToSchema)]
pub struct MatchVeto {
    pub match_id: u64,
    pub team_id: u64,

    pub map_name: String,
    pub r#type: String,
    pub order: i32,
}