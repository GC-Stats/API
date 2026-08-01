/*
    GC-Stats — API

    Builds the system prompt handed to the LLM provider: instructions plus
    the catalogue of measures/dimensions pulled from the real Cube schema,
    so the model only ever sees fields that actually exist.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use crate::nlquery::cube::CubeSchemaSet;

pub struct SystemPrompt {
    pub instructions: String,
    pub catalog: String,
}

impl SystemPrompt {
    pub fn flat(&self) -> String {
        format!("{}\n\n{}", self.instructions, self.catalog)
    }
}

pub fn build_system_prompt(schema: &CubeSchemaSet) -> SystemPrompt {
    let mut measures: Vec<&String> = schema.measures.iter().collect();
    measures.sort();
    let mut dimensions: Vec<&String> = schema.dimensions.iter().collect();
    dimensions.sort();

    let instructions = "You are a query planner for a Cube.dev semantic layer used by GC Stats, \
a Valorant esports stats platform. Translate the user's natural-language \
question into a Cube query by calling the build_cube_query tool.\n\n\
Only use measure, dimension and filter member names from the lists below — \
never invent a field that isn't listed. If the question cannot be answered \
with these fields, pick the closest reasonable interpretation.\n\n\
IMPORTANT — every measure, dimension and filter member in a single query must \
come from the SAME view (the part before the first '.', e.g. all \
\"player_stats.*\" or all \"kill_stats.*\" — never a mix of the two). Views are \
independent rollups that are never joined to one another, so combining fields \
from two different views always fails. If a question needs several figures \
together (e.g. kills, deaths, assists, ACS, ADR in one row per player), pick \
the single view that covers all of them — that's player_stats, not \
kill_stats: kill_stats only has kill_count, weapon and kill-event fields. \
Reach for kill_stats only when the question is purely about kill events \
(weapon, headshot, who killed whom) and doesn't also need deaths, assists, \
ACS, ADR or KAST — those only exist on player_stats. If a question combines a \
situational stat (pistol/eco/force/full-buy/post-plant/clutch win rate, \
multikills, trades) with a core stat (kills, deaths, assists, ACS, ADR, \
KAST, headshot%), use player_advanced_stats — it carries both, with the \
core stats prefixed player_advanced_stats.game_player_stats_*.\n\n\
IMPORTANT — this data is very large: every query MUST include at least one \
of these narrowing filters, or it will be rejected:\n\
- a filter on a tournament/team/player-identifying field (operator \"equals\"), e.g. \
player_stats.tournament_id, player_stats.player_id, kill_stats.matches_tournament_id, \
kill_stats.killer_players_handle, or kill_stats.victim_players_handle\n\
- a filter on that view's created_at with operator \"inDateRange\" spanning at most 1 year\n\n\
If the user's question doesn't name a team, player or tournament, default to a \
created_at inDateRange filter for the last 90 days rather than leaving the \
query unscoped."
        .to_string();

    let catalog = format!(
        "Available measures:\n{}\n\n\
Available dimensions:\n{}",
        measures.iter().map(|m| format!("- {m}")).collect::<Vec<_>>().join("\n"),
        dimensions.iter().map(|d| format!("- {d}")).collect::<Vec<_>>().join("\n"),
    );

    SystemPrompt { instructions, catalog }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn build_system_prompt_lists_measures_and_dimensions() {
        let schema = CubeSchemaSet {
            measures: HashSet::from(["Kills.count".to_string()]),
            dimensions: HashSet::from(["Players.name".to_string(), "Maps.name".to_string()]),
        };

        let prompt = build_system_prompt(&schema).flat();

        assert!(prompt.contains("- Kills.count"));
        assert!(prompt.contains("- Players.name"));
        assert!(prompt.contains("- Maps.name"));
        assert!(prompt.contains("build_cube_query"));
    }

    #[test]
    fn build_system_prompt_sorts_fields_deterministically() {
        let schema = CubeSchemaSet {
            measures: HashSet::from(["Zebra.count".to_string(), "Apple.count".to_string()]),
            dimensions: HashSet::new(),
        };

        let prompt = build_system_prompt(&schema).flat();
        let apple_pos = prompt.find("Apple.count").unwrap();
        let zebra_pos = prompt.find("Zebra.count").unwrap();
        assert!(apple_pos < zebra_pos);
    }

    #[test]
    fn build_system_prompt_splits_instructions_from_catalog() {
        let schema = CubeSchemaSet {
            measures: HashSet::from(["Kills.count".to_string()]),
            dimensions: HashSet::new(),
        };

        let prompt = build_system_prompt(&schema);

        assert!(prompt.instructions.contains("build_cube_query"));
        assert!(!prompt.instructions.contains("Kills.count"));
        assert!(prompt.catalog.contains("- Kills.count"));
        assert!(!prompt.catalog.contains("build_cube_query"));
    }
}
