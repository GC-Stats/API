/*
    GC-Stats — API

    Shared response models and helpers for players and teams: logo/photo URL
    building, logo history partitioning, batched current-logo lookups, and
    the `Player`/`Team` DB row types.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use sqlx::FromRow;
use sqlx::mysql::MySql;
use sqlx::QueryBuilder;
use serde_json::Value;
use utoipa::ToSchema;
use chrono::{DateTime, Utc};

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct LogoUrls {
    pub url_200x200: String,
    pub url_full: String,
}

impl LogoUrls {
    pub fn build(entity_type: &str, uuid: &str) -> Self {
        Self {
            url_200x200: format!("https://gc-stats.app/storage/{}/{}/200x200.webp", entity_type, uuid),
            url_full: format!("https://gc-stats.app/storage/{}/{}/full.webp", entity_type, uuid),
        }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LogoEntry {
    pub uuid: String,
    pub url_200x200: String,
    pub url_full: String,
    pub from: DateTime<Utc>,
    pub until: Option<DateTime<Utc>>,
}

#[derive(Serialize, ToSchema)]
pub struct LogoHistoryResponse {
    pub current: Option<LogoEntry>,
    pub history: Vec<LogoEntry>,
}

#[derive(Debug, FromRow)]
pub struct LogoRow {
    pub id: String,
    pub from: DateTime<Utc>,
    pub until: Option<DateTime<Utc>>,
}

impl LogoRow {
    pub fn into_entry(self, entity_type: &str) -> LogoEntry {
        let urls = LogoUrls::build(entity_type, &self.id);
        LogoEntry {
            uuid: self.id,
            url_200x200: urls.url_200x200,
            url_full: urls.url_full,
            from: self.from,
            until: self.until,
        }
    }
}

/// Splits logo rows into the current entry (`until IS NULL`) and past entries.
pub fn partition_logo_history(rows: Vec<LogoRow>, entity_type: &str) -> LogoHistoryResponse {
    let mut current = None;
    let mut history = Vec::new();

    for row in rows {
        let entry = row.into_entry(entity_type);
        if entry.until.is_none() {
            current = Some(entry);
        } else {
            history.push(entry);
        }
    }

    LogoHistoryResponse { current, history }
}

/// Fetches the current (`until IS NULL`) logo/photo uuid for a batch of entities
/// in a single query, keyed by entity id. Failures are logged and treated as
/// "no logo" so a missing image never fails the whole endpoint.
pub async fn fetch_current_logo_ids(
    db: &sqlx::MySqlPool,
    entity_type: &str,
    entity_ids: &[u64],
) -> HashMap<u64, String> {
    if entity_ids.is_empty() {
        return HashMap::new();
    }

    let mut qb: QueryBuilder<MySql> = QueryBuilder::new(
        "SELECT entity_id, id FROM logos WHERE until IS NULL AND entity_type = "
    );
    qb.push_bind(entity_type);
    qb.push(" AND entity_id IN (");
    let mut ids = qb.separated(", ");
    for id in entity_ids {
        ids.push_bind(*id);
    }
    qb.push(")");

    match qb.build_query_as::<(u64, String)>().fetch_all(db).await {
        Ok(rows) => rows.into_iter().collect(),
        Err(err) => {
            tracing::error!("DB error fetching {} logos: {:?}", entity_type, err);
            HashMap::new()
        }
    }
}

#[derive(Serialize, ToSchema)]
pub struct TeamResponse {
    #[serde(flatten)]
    pub team: Team,
    pub logo: Option<LogoUrls>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct PlayerFullResponse {
    #[serde(flatten)]
    pub player: crate::models::entity::Player,
    pub current_team: Option<crate::models::entity::Team>,
    pub photo: Option<LogoUrls>,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct PlayerTeamHistory {
    pub team_id: u64,
    pub team_name: String,
    pub team_short_name: Option<String>,
    pub team_country: Option<String>,
    pub role: String,
    pub joined_at: chrono::NaiveDate,
    pub left_at: Option<chrono::NaiveDate>,
}

#[derive(Debug, FromRow, Serialize, Deserialize, ToSchema)]
pub struct Player {
    pub id: u64,
    pub handle: String,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub country_code: Option<String>,
    pub bio: Option<String>,

    #[sqlx(json)]
    #[schema(value_type = Object)]
    pub socials: Value,

    pub vlr_id: Option<i32>,
    pub is_active: bool,
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct TeamPlayersResponse {
    pub current: Vec<crate::models::entity::Player>,
    pub history: Vec<crate::models::entity::Player>,
}

#[derive(Debug, FromRow, Serialize, Deserialize, ToSchema)]
pub struct Team {
    pub id: u64,
    pub name: String,
    pub short_name: Option<String>,
    pub country_code: Option<String>,

    #[sqlx(json)]
    #[schema(value_type = Object)]
    pub socials: Value,
    
    pub bio: Option<String>,
    pub vlr_id: Option<i32>,
    pub is_active: bool,
}

impl Team {
    /// Builds a `Team` from the nullable columns of a `LEFT JOIN teams` row,
    /// where every column except the (already unwrapped) id may be NULL.
    #[allow(clippy::too_many_arguments)]
    pub fn from_joined_row(
        id: u64,
        name: Option<String>,
        short_name: Option<String>,
        country_code: Option<String>,
        socials: Option<&str>,
        bio: Option<String>,
        vlr_id: Option<i32>,
        is_active: Option<i8>,
    ) -> Self {
        Self {
            id,
            name: name.unwrap_or_default(),
            short_name,
            country_code,
            socials: socials.map(parse_socials).unwrap_or_else(|| serde_json::json!({})),
            bio,
            vlr_id,
            is_active: is_active.unwrap_or(0) != 0,
        }
    }
}

/// The `socials` column is stored as raw text, not a typed JSON column, so
/// manually-constructed `Team` values (outside of `query_as::<_, Team>`,
/// which benefits from `#[sqlx(json)]`) need to parse it explicitly.
pub fn parse_socials(raw: &str) -> Value {
    serde_json::from_str(raw).unwrap_or_else(|_| serde_json::json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_socials_falls_back_to_empty_object() {
        assert_eq!(parse_socials("not json"), serde_json::json!({}));
        assert_eq!(
            parse_socials(r#"{"twitter":"@gc"}"#),
            serde_json::json!({"twitter": "@gc"})
        );
    }

    #[test]
    fn team_from_joined_row_defaults_nullable_columns() {
        let team = Team::from_joined_row(7, None, None, None, None, None, None, None);
        assert_eq!(team.id, 7);
        assert_eq!(team.name, "");
        assert_eq!(team.socials, serde_json::json!({}));
        assert!(!team.is_active);
    }
}

#[derive(serde::Serialize, utoipa::ToSchema)]
pub struct TeamWithScore {
    #[serde(flatten)]
    pub team: crate::models::entity::Team,

    pub score: Option<i32>,
}