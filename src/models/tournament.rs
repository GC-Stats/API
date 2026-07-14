/*
    GC-Stats — API

    Response models for tournaments: the tournament record and its phases.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use serde::{Serialize, Deserialize};
use sqlx::FromRow;
use chrono::NaiveDate;
use crate::models::entity::LogoUrls;

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct TournamentFullResponse {
    #[serde(flatten)]
    pub tournament: crate::models::tournament::Tournament,
    pub phases: Vec<crate::models::tournament::TournamentPhase>,
    pub logo: Option<LogoUrls>,
}

#[derive(Debug, FromRow, Serialize, Deserialize, utoipa::ToSchema)]
pub struct Tournament {
    pub id: u64,
    pub name: String,
    pub region: String,
    pub category: String,
    pub prize_pool: Option<String>,
    pub location: Option<String>,
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub status: String,
    pub description: Option<String>,
}

#[derive(Debug, FromRow, Serialize, Deserialize, utoipa::ToSchema)]
pub struct TournamentPhase {
    pub id: u64,
    pub tournament_id: u64,
    pub name: String,
    pub format: Option<String>,
    pub parent_id: Option<u64>,
    pub match_ids: Vec<i64>,
}