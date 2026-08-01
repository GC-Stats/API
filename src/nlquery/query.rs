/*
    GC-Stats — API

    The Cube.dev query shape produced by the LLM (measures/dimensions/
    filters), the JSON schema handed to providers for structured output,
    validation against the real Cube schema before anything gets executed,
    and a scope guard forcing large per-round/per-map stats queries to be
    narrowed down before they ever reach the DB.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use chrono::{NaiveDate, NaiveDateTime};
use serde::{Deserialize, Serialize};

use crate::nlquery::cube::CubeSchemaSet;
use crate::nlquery::error::NlQueryError;

/// Cubes big enough (hundreds of thousands to millions of rows — round
/// kills, damages, alive-state snapshots, per-round/per-map player stats,
/// and the long-format team-results cubes built on top of them) that an
/// unscoped query is a real risk to the DB, especially now that they're
/// all directly joinable to each other. Every query touching one of these
/// must be narrowed down — see validate_query_scope(). Also includes every
/// LLM-safe view built on top of one of these raw cubes (kill_stats,
/// player_stats, player_advanced_stats, damage_stats, round_results,
/// map_results, match_results, round_state_stats, player_positions) —
/// nl-query only ever references view-qualified member names, never the
/// raw cube names directly, so without the view name here too this guard
/// would silently do nothing for the AI query path.
const RESTRICTED_CUBE_NAMES: &[&str] = &[
    "kill_stats",
    "game_map_rounds",
    "game_map_round_kills",
    "game_map_round_damages",
    "game_map_round_alive_states",
    "game_map_round_player_positions",
    "game_map_round_player_stats",
    "game_maps",
    "game_player_stats",
    "game_player_advanced_stats",
    "match_team_results",
    "map_team_results",
    "round_team_results",
    "player_stats",
    "player_advanced_stats",
    "damage_stats",
    "round_results",
    "map_results",
    "match_results",
    "round_state_stats",
    "player_positions",
];

/// A filter member CONTAINING one of these is treated as a valid "narrow it
/// down" filter, as long as it's pinned to one value (operator "equals").
/// Substring rather than an exact ".field_name" suffix on purpose: Cube
/// views alias joined-in fields with a prefix (e.g. kill_stats exposes
/// tournament_id as "matches_tournament_id", not "tournament_id"), so a
/// strict suffix check would silently never match on the one view nl-query
/// actually uses. "_handle" covers kill_stats' player fields
/// (killer_players_handle/victim_players_handle) — that view has no
/// numeric player_id, a player is scoped by name there.
const SCOPE_ID_SUBSTRINGS: &[&str] = &["team_id", "opponent_id", "player_id", "tournament_id", "_handle"];

const MAX_DATE_RANGE_DAYS: i64 = 366;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CubeFilter {
    pub member: String,
    pub operator: String,
    #[serde(default)]
    pub values: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CubeQuery {
    #[serde(default)]
    pub measures: Vec<String>,
    #[serde(default)]
    pub dimensions: Vec<String>,
    #[serde(default)]
    pub filters: Vec<CubeFilter>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit: Option<u32>,
}

/// JSON schema describing `CubeQuery`, handed to the LLM provider as the
/// tool/function parameters it must fill in — the model never gets to
/// return free text for this part.
pub fn cube_query_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "measures": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Names from the \"Available measures\" list only — numeric aggregates \
                    (counts, sums, averages...) such as kills, deaths, assists, ACS, ADR. Never put a \
                    measure name here if it's a dimension instead, or vice versa."
            },
            "dimensions": {
                "type": "array",
                "items": { "type": "string" },
                "description": "Names from the \"Available dimensions\" list only — groupings/labels \
                    (player handle, map name, team name...), never a numeric aggregate. Never put a \
                    measure name here."
            },
            "filters": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "member": { "type": "string" },
                        "operator": { "type": "string" },
                        "values": { "type": "array", "items": { "type": "string" } }
                    },
                    "required": ["member", "operator", "values"]
                }
            },
            "limit": { "type": ["integer", "null"] }
        },
        "required": ["measures", "dimensions", "filters"]
    })
}

/// Moves any member the model put in the wrong bucket (a real measure listed
/// under `dimensions`, or a real dimension listed under `measures`) into its
/// correct bucket, deduplicating afterwards. Models occasionally get this
/// split wrong even though the field name itself is valid — e.g. asking for
/// `player_stats.total_kills` (a measure) as a dimension. Call this before
/// validate_cube_query() so that mistake doesn't hard-fail an otherwise
/// answerable query; a name in neither list is left alone and still rejected
/// by validate_cube_query() as a genuine hallucination.
pub fn normalize_measure_dimension_split(query: &mut CubeQuery, schema: &CubeSchemaSet) {
    let mut measures = Vec::with_capacity(query.measures.len() + query.dimensions.len());
    let mut dimensions = Vec::with_capacity(query.dimensions.len() + query.measures.len());

    for member in query.measures.drain(..).chain(query.dimensions.drain(..)) {
        if schema.measures.contains(&member) {
            measures.push(member);
        } else if schema.dimensions.contains(&member) {
            dimensions.push(member);
        } else {
            // Unknown to both — leave it where validate_cube_query() will
            // report it; keep it in measures only so it's reported once.
            measures.push(member);
        }
    }

    measures.sort();
    measures.dedup();
    dimensions.sort();
    dimensions.dedup();

    query.measures = measures;
    query.dimensions = dimensions;
}

/// Rejects a query that references a measure/dimension/filter member absent
/// from the real Cube schema (the model hallucinated a field), or that asks
/// for nothing at all. Run normalize_measure_dimension_split() first so a
/// valid field merely placed in the wrong bucket isn't reported as unknown.
pub fn validate_cube_query(query: &CubeQuery, schema: &CubeSchemaSet) -> Result<(), NlQueryError> {
    if query.measures.is_empty() && query.dimensions.is_empty() {
        return Err(NlQueryError::InvalidCubeQuery(
            "query must include at least one measure or dimension".to_string(),
        ));
    }

    for measure in &query.measures {
        if !schema.measures.contains(measure) {
            return Err(NlQueryError::InvalidCubeQuery(format!("unknown measure: {measure}")));
        }
    }

    for dimension in &query.dimensions {
        if !schema.dimensions.contains(dimension) {
            return Err(NlQueryError::InvalidCubeQuery(format!("unknown dimension: {dimension}")));
        }
    }

    for filter in &query.filters {
        if !schema.measures.contains(&filter.member) && !schema.dimensions.contains(&filter.member) {
            return Err(NlQueryError::InvalidCubeQuery(format!("unknown filter member: {}", filter.member)));
        }
    }

    Ok(())
}

fn cube_name_of(member: &str) -> &str {
    member.split('.').next().unwrap_or(member)
}

/// Rejects a query that mixes members from more than one LLM-safe view
/// (e.g. kill_stats.kill_count alongside player_stats.total_deaths). Each
/// view is its own self-contained rollup with its own join tree — views are
/// never joined to one another — so a cross-view query isn't just
/// unanswerable, it's a Cube "Can't find join path" 400 that would otherwise
/// only surface after the round-trip to Cube. When a question needs several
/// of these figures together (kills+deaths+assists+ACS+ADR), the model must
/// pick the single view that covers all of them (player_stats) rather than
/// combining views — see the "only use one view per query" instruction in
/// build_system_prompt(). Only meant for the nl-query path: the human query
/// builder queries get_full_cube_schema()'s raw cubes directly, which
/// genuinely do join to each other.
pub fn validate_single_view(query: &CubeQuery) -> Result<(), NlQueryError> {
    let mut views = query
        .measures
        .iter()
        .chain(query.dimensions.iter())
        .chain(query.filters.iter().map(|f| &f.member))
        .map(|member| cube_name_of(member));

    let Some(first) = views.next() else { return Ok(()) };

    if let Some(other) = views.find(|v| *v != first) {
        return Err(NlQueryError::InvalidCubeQuery(format!(
            "query mixes fields from two different views ('{first}' and '{other}') — views can't be \
             joined together, pick the single view that covers everything the question needs"
        )));
    }

    Ok(())
}

fn touches_restricted_cube(query: &CubeQuery) -> bool {
    query
        .measures
        .iter()
        .chain(query.dimensions.iter())
        .chain(query.filters.iter().map(|f| &f.member))
        .any(|member| RESTRICTED_CUBE_NAMES.contains(&cube_name_of(member)))
}

fn has_id_scope_filter(query: &CubeQuery) -> bool {
    query.filters.iter().any(|f| {
        f.operator == "equals"
            && !f.values.is_empty()
            && SCOPE_ID_SUBSTRINGS.iter().any(|needle| f.member.contains(needle))
    })
}

/// Cube's date strings can be a bare date ("2025-01-01") or a full
/// timestamp ("2025-01-01T00:00:00.000") depending on what produced them —
/// accept either.
fn parse_loose_date(value: &str) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .or_else(|| NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M:%S%.f").ok().map(|dt| dt.date()))
}

fn has_bounded_date_range_filter(query: &CubeQuery) -> bool {
    query.filters.iter().any(|f| {
        if f.operator != "inDateRange" || f.values.len() != 2 {
            return false;
        }

        match (parse_loose_date(&f.values[0]), parse_loose_date(&f.values[1])) {
            (Some(start), Some(end)) => (end - start).num_days().abs() <= MAX_DATE_RANGE_DAYS,
            _ => false,
        }
    })
}

/// Forces every query touching a RESTRICTED_CUBE_NAMES cube to be narrowed
/// down by team, player, tournament, or a date range of at most one year —
/// otherwise it's a full scan across hundreds of thousands to millions of
/// rows, now made worse by every round-level cube being directly joinable
/// to every other one (see the Cube model's join comments).
pub fn validate_query_scope(query: &CubeQuery) -> Result<(), NlQueryError> {
    if !touches_restricted_cube(query) {
        return Ok(());
    }

    if has_id_scope_filter(query) || has_bounded_date_range_filter(query) {
        return Ok(());
    }

    Err(NlQueryError::InvalidCubeQuery(
        "This query touches large stats tables and must be narrowed down with a filter on team, \
         player, or tournament (operator \"equals\"), or a date range of at most 1 year (operator \
         \"inDateRange\")"
            .to_string(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn mock_schema() -> CubeSchemaSet {
        CubeSchemaSet {
            measures: HashSet::from(["Kills.count".to_string(), "Rounds.count".to_string()]),
            dimensions: HashSet::from(["Players.name".to_string(), "Maps.name".to_string()]),
        }
    }

    #[test]
    fn validate_cube_query_accepts_known_members() {
        let query = CubeQuery {
            measures: vec!["Kills.count".to_string()],
            dimensions: vec!["Players.name".to_string()],
            filters: vec![CubeFilter {
                member: "Maps.name".to_string(),
                operator: "equals".to_string(),
                values: vec!["Sova".to_string()],
            }],
            limit: None,
        };

        assert!(validate_cube_query(&query, &mock_schema()).is_ok());
    }

    #[test]
    fn normalize_measure_dimension_split_moves_a_measure_out_of_dimensions() {
        let mut query = CubeQuery {
            measures: vec![],
            dimensions: vec!["Kills.count".to_string(), "Players.name".to_string()],
            filters: vec![],
            limit: None,
        };

        normalize_measure_dimension_split(&mut query, &mock_schema());

        assert_eq!(query.measures, vec!["Kills.count".to_string()]);
        assert_eq!(query.dimensions, vec!["Players.name".to_string()]);
    }

    #[test]
    fn normalize_measure_dimension_split_moves_a_dimension_out_of_measures() {
        let mut query = CubeQuery {
            measures: vec!["Kills.count".to_string(), "Players.name".to_string()],
            dimensions: vec![],
            filters: vec![],
            limit: None,
        };

        normalize_measure_dimension_split(&mut query, &mock_schema());

        assert_eq!(query.measures, vec!["Kills.count".to_string()]);
        assert_eq!(query.dimensions, vec!["Players.name".to_string()]);
    }

    #[test]
    fn normalize_measure_dimension_split_leaves_hallucinated_fields_for_validation_to_catch() {
        let mut query = CubeQuery {
            measures: vec![],
            dimensions: vec!["Kills.headshots".to_string()],
            filters: vec![],
            limit: None,
        };

        normalize_measure_dimension_split(&mut query, &mock_schema());

        let err = validate_cube_query(&query, &mock_schema()).unwrap_err();
        assert!(matches!(err, NlQueryError::InvalidCubeQuery(msg) if msg.contains("Kills.headshots")));
    }

    #[test]
    fn validate_cube_query_rejects_hallucinated_measure() {
        let query = CubeQuery {
            measures: vec!["Kills.headshots".to_string()],
            dimensions: vec![],
            filters: vec![],
            limit: None,
        };

        let err = validate_cube_query(&query, &mock_schema()).unwrap_err();
        assert!(matches!(err, NlQueryError::InvalidCubeQuery(msg) if msg.contains("Kills.headshots")));
    }

    #[test]
    fn validate_cube_query_rejects_hallucinated_filter_member() {
        let query = CubeQuery {
            measures: vec!["Kills.count".to_string()],
            dimensions: vec![],
            filters: vec![CubeFilter {
                member: "Weapons.name".to_string(),
                operator: "equals".to_string(),
                values: vec!["Vandal".to_string()],
            }],
            limit: None,
        };

        let err = validate_cube_query(&query, &mock_schema()).unwrap_err();
        assert!(matches!(err, NlQueryError::InvalidCubeQuery(msg) if msg.contains("Weapons.name")));
    }

    #[test]
    fn validate_cube_query_rejects_empty_query() {
        let query = CubeQuery { measures: vec![], dimensions: vec![], filters: vec![], limit: None };

        let err = validate_cube_query(&query, &mock_schema()).unwrap_err();
        assert!(matches!(err, NlQueryError::InvalidCubeQuery(_)));
    }

    #[test]
    fn validate_query_scope_ignores_queries_that_never_touch_a_restricted_cube() {
        let query = CubeQuery {
            measures: vec!["matches.count".to_string()],
            dimensions: vec!["teams.name".to_string()],
            filters: vec![],
            limit: None,
        };

        assert!(validate_query_scope(&query).is_ok());
    }

    #[test]
    fn validate_query_scope_rejects_an_unscoped_restricted_query() {
        let query = CubeQuery {
            measures: vec!["game_map_round_player_stats.count".to_string()],
            dimensions: vec!["game_map_round_player_stats.weapon_id".to_string()],
            filters: vec![],
            limit: None,
        };

        let err = validate_query_scope(&query).unwrap_err();
        assert!(matches!(err, NlQueryError::InvalidCubeQuery(_)));
    }

    #[test]
    fn validate_query_scope_accepts_a_team_id_filter() {
        let query = CubeQuery {
            measures: vec!["game_map_round_player_stats.count".to_string()],
            dimensions: vec![],
            filters: vec![CubeFilter {
                member: "game_map_round_player_stats.team_id".to_string(),
                operator: "equals".to_string(),
                values: vec!["42".to_string()],
            }],
            limit: None,
        };

        assert!(validate_query_scope(&query).is_ok());
    }

    #[test]
    fn validate_query_scope_rejects_a_non_equals_team_id_filter() {
        let query = CubeQuery {
            measures: vec!["game_map_round_player_stats.count".to_string()],
            dimensions: vec![],
            filters: vec![CubeFilter {
                member: "game_map_round_player_stats.team_id".to_string(),
                operator: "notEquals".to_string(),
                values: vec!["42".to_string()],
            }],
            limit: None,
        };

        assert!(validate_query_scope(&query).is_err());
    }

    #[test]
    fn validate_query_scope_accepts_a_bounded_date_range() {
        let query = CubeQuery {
            measures: vec!["game_maps.count".to_string()],
            dimensions: vec![],
            filters: vec![CubeFilter {
                member: "game_maps.created_at".to_string(),
                operator: "inDateRange".to_string(),
                values: vec!["2025-01-01".to_string(), "2025-06-01".to_string()],
            }],
            limit: None,
        };

        assert!(validate_query_scope(&query).is_ok());
    }

    #[test]
    fn validate_query_scope_rejects_a_date_range_over_one_year() {
        let query = CubeQuery {
            measures: vec!["game_maps.count".to_string()],
            dimensions: vec![],
            filters: vec![CubeFilter {
                member: "game_maps.created_at".to_string(),
                operator: "inDateRange".to_string(),
                values: vec!["2020-01-01".to_string(), "2025-01-01".to_string()],
            }],
            limit: None,
        };

        assert!(validate_query_scope(&query).is_err());
    }

    #[test]
    fn validate_query_scope_accepts_full_timestamp_date_values() {
        let query = CubeQuery {
            measures: vec!["game_maps.count".to_string()],
            dimensions: vec![],
            filters: vec![CubeFilter {
                member: "game_maps.created_at".to_string(),
                operator: "inDateRange".to_string(),
                values: vec!["2025-01-01T00:00:00.000".to_string(), "2025-03-01T23:59:59.999".to_string()],
            }],
            limit: None,
        };

        assert!(validate_query_scope(&query).is_ok());
    }

    #[test]
    fn validate_query_scope_accepts_a_player_id_or_tournament_id_filter() {
        let by_player = CubeQuery {
            measures: vec!["game_player_stats.count".to_string()],
            dimensions: vec![],
            filters: vec![CubeFilter {
                member: "game_player_stats.player_id".to_string(),
                operator: "equals".to_string(),
                values: vec!["7".to_string()],
            }],
            limit: None,
        };
        assert!(validate_query_scope(&by_player).is_ok());

        let by_tournament = CubeQuery {
            measures: vec!["game_player_stats.count".to_string()],
            dimensions: vec![],
            filters: vec![CubeFilter {
                member: "game_player_stats.tournament_id".to_string(),
                operator: "equals".to_string(),
                values: vec!["3".to_string()],
            }],
            limit: None,
        };
        assert!(validate_query_scope(&by_tournament).is_ok());
    }

    #[test]
    fn validate_query_scope_accepts_kill_stats_prefixed_view_field_names() {
        // kill_stats (the actual view nl-query queries) aliases joined-in
        // fields with a prefix — "matches_tournament_id", not a bare
        // "tournament_id" — and has no numeric player_id at all, only
        // killer_players_handle/victim_players_handle. Both must count as
        // valid scoping.
        let by_tournament = CubeQuery {
            measures: vec!["kill_stats.kill_count".to_string()],
            dimensions: vec![],
            filters: vec![CubeFilter {
                member: "kill_stats.matches_tournament_id".to_string(),
                operator: "equals".to_string(),
                values: vec!["3".to_string()],
            }],
            limit: None,
        };
        assert!(validate_query_scope(&by_tournament).is_ok());

        let by_player_handle = CubeQuery {
            measures: vec!["kill_stats.kill_count".to_string()],
            dimensions: vec![],
            filters: vec![CubeFilter {
                member: "kill_stats.killer_players_handle".to_string(),
                operator: "equals".to_string(),
                values: vec!["Player1".to_string()],
            }],
            limit: None,
        };
        assert!(validate_query_scope(&by_player_handle).is_ok());

        let unscoped = CubeQuery {
            measures: vec!["kill_stats.kill_count".to_string()],
            dimensions: vec!["kill_stats.weapon".to_string()],
            filters: vec![],
            limit: None,
        };
        assert!(validate_query_scope(&unscoped).is_err());
    }

    #[test]
    fn validate_single_view_accepts_a_query_confined_to_one_view() {
        let query = CubeQuery {
            measures: vec!["player_stats.total_kills".to_string(), "player_stats.total_deaths".to_string()],
            dimensions: vec!["player_stats.players_handle".to_string()],
            filters: vec![CubeFilter {
                member: "player_stats.tournament_id".to_string(),
                operator: "equals".to_string(),
                values: vec!["261".to_string()],
            }],
            limit: None,
        };

        assert!(validate_single_view(&query).is_ok());
    }

    #[test]
    fn validate_single_view_rejects_a_query_mixing_two_views() {
        let query = CubeQuery {
            measures: vec!["kill_stats.kill_count".to_string(), "player_stats.total_deaths".to_string()],
            dimensions: vec![],
            filters: vec![],
            limit: None,
        };

        let err = validate_single_view(&query).unwrap_err();
        assert!(matches!(err, NlQueryError::InvalidCubeQuery(msg) if msg.contains("kill_stats") && msg.contains("player_stats")));
    }

    #[test]
    fn validate_query_scope_restricts_every_llm_safe_stats_view() {
        for view in [
            "player_stats",
            "player_advanced_stats",
            "damage_stats",
            "round_results",
            "map_results",
            "match_results",
            "round_state_stats",
            "player_positions",
        ] {
            let unscoped = CubeQuery {
                measures: vec![format!("{view}.count")],
                dimensions: vec![],
                filters: vec![],
                limit: None,
            };
            assert!(validate_query_scope(&unscoped).is_err(), "{view} should require scoping");

            let scoped = CubeQuery {
                measures: vec![format!("{view}.count")],
                dimensions: vec![],
                filters: vec![CubeFilter {
                    member: format!("{view}.tournament_id"),
                    operator: "equals".to_string(),
                    values: vec!["261".to_string()],
                }],
                limit: None,
            };
            assert!(validate_query_scope(&scoped).is_ok(), "{view} should accept a tournament_id filter");
        }
    }
}
