/*
    GC-Stats — API

    OpenAPI documentation for the public API. Declares the documented paths,
    response schemas and tags, and registers the `x-api-key` header security
    scheme shown in Swagger UI.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use utoipa::openapi::security::{ApiKey, ApiKeyValue, SecurityScheme};
use utoipa::{Modify, OpenApi};
use crate::routes::{matches, players, tournaments, map};
use crate::routes::teams;

#[derive(OpenApi)]
#[openapi(
    paths(
        players::get_player,
        players::get_player_teams,
        players::get_player_by_name,
        players::get_player_photos,
        players::get_player_stats,
        players::get_player_agents,
        players::get_player_weapons,
        teams::get_team,
        teams::get_team_players,
        teams::get_team_by_name,
        teams::get_team_logos,
        teams::get_team_stats,
        teams::get_team_maps,
        teams::get_team_weapons,
        tournaments::get_tournament,
        tournaments::get_tournament_by_name,
        tournaments::get_tournament_logos,
        matches::get_match,
        matches::get_match_v2,
        map::get_map,
        map::get_map_rounds,
    ),
    components(
        schemas(
            crate::models::entity::LogoUrls,
            crate::models::entity::LogoEntry,
            crate::models::entity::LogoHistoryResponse,
            crate::models::entity::PlayerFullResponse,
            crate::models::entity::PlayerTeamHistory,
            crate::models::entity::Player,
            crate::models::entity::TeamPlayersResponse,
            crate::models::entity::Team,
            crate::models::entity::TeamResponse,
            crate::models::entity::TeamWithScore,
            crate::models::tournament::TournamentFullResponse,
            crate::models::tournament::Tournament,
            crate::models::tournament::TournamentPhase,
            crate::models::matchs::MatchFullResponse,
            crate::models::matchs::Match,
            crate::models::matchs::MatchVeto,
            crate::models::game::MapFullResponse,
            crate::models::game::GameMap,
            crate::models::game::GamePlayerStat,
            crate::models::game::RoundFullResponse,
            crate::models::game::RoundPlayerStat,
            crate::models::stats::AvgStatsResponse,
            crate::models::stats::SideStats,
            crate::models::stats::SideAvg,
            crate::models::stats::Side,
            crate::models::stats::TeamStatsResponse,
            crate::models::stats::SideSituations,
            crate::models::stats::SituationEntry,
            crate::models::stats::SidePostPlant,
            crate::models::stats::PostPlantStats,
            crate::models::stats::SideWinrate,
            crate::models::stats::TeamMapEntry,
            crate::models::stats::CompEntry,
            crate::models::stats::AgentStatsEntry,
            crate::models::stats::WeaponStatsEntry,
            crate::models::stats::MatchStatsResponse,
            crate::models::stats::MapStatsFull,
            crate::models::stats::RoundStatsFull,
            crate::models::stats::RoundPlayerStatFull,
            crate::models::stats::RoundKillEvent,
        )
    ),
    tags(
        (name = "Players", description = "Retrieve players and their team history"),
        (name = "Teams", description = "Retrieve teams and their player history"),
        (name = "Tournaments", description = "Retrieve tournaments, phase and matches"),
        (name = "Matches", description = "Retrieve matches, vetos, maps & stats"),
        (name = "Maps", description = "Retrieve map result, stats & round details"),
    ),
    modifiers(&SecurityAddon),
    security(("api_key" = [])),
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "api_key",
                SecurityScheme::ApiKey(ApiKey::Header(ApiKeyValue::new("x-api-key"))),
            )
        }
    }
}
