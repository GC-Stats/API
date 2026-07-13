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