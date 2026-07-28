/*
    GC-Stats — API

    Response models and DB access for aggregate performance stats: average
    stats always split by side (atk/def), maps played (with nested comps and
    atk/def winrate), agent stats, round situations (XvY) and post-plant
    stats for teams.

    Copyright (c) 2026 Alice Alleman — GC-Stats-API
    License: https://github.com/GC-Stats/API/blob/main/LICENSE.md (GC-Stats License v1.0)
    Repository: https://github.com/GC-Stats/API
*/

use std::collections::HashMap;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, MySql, MySqlPool, QueryBuilder};
use utoipa::{IntoParams, ToSchema};
use crate::models::entity::{Team, TeamWithScore};
use crate::models::game::{GameMap, GamePlayerStat};
use crate::models::matchs::{Match, MatchVeto};

/// Which id column of `game_player_stats` an aggregate query is scoped to.
#[derive(Debug, Clone, Copy)]
pub enum EntityKind {
    Player,
    Team,
}

impl EntityKind {
    fn gps_column(self) -> &'static str {
        match self {
            EntityKind::Player => "gps.player_id",
            EntityKind::Team => "gps.team_id",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Atk,
    Def,
}

impl Side {
    fn side_team_column(self) -> &'static str {
        match self {
            Side::Atk => "r.atk_team",
            Side::Def => "r.def_team",
        }
    }
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct StatsQuery {
    /// Only include maps from matches scheduled on/after this date.
    pub from: Option<NaiveDate>,
    /// Only include maps from matches scheduled on/before this date.
    pub to: Option<NaiveDate>,
    /// Only include stats recorded on this agent.
    pub agent: Option<String>,
    /// Only include stats from this tournament.
    pub tournament_id: Option<u64>,
}

#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct MapsQuery {
    /// Scope maps + comps to a single tournament instead of all-time.
    pub tournament_id: Option<u64>,
}

fn push_common_filters(qb: &mut QueryBuilder<MySql>, filters: &StatsQuery, tournament_column: &str) {
    if let Some(from) = filters.from {
        qb.push(" AND m.scheduled_at >= ").push_bind(from);
    }
    if let Some(to) = filters.to {
        qb.push(" AND m.scheduled_at <= ").push_bind(to);
    }
    if let Some(agent) = &filters.agent {
        qb.push(" AND gps.agent_name = ").push_bind(agent.clone());
    }
    if let Some(tournament_id) = filters.tournament_id {
        qb.push(format!(" AND {tournament_column} = "));
        qb.push_bind(tournament_id);
    }
}

/// Same as `push_common_filters` but for round-only queries that don't join
/// `game_player_stats`, so there's no `agent` to filter on.
fn push_round_filters(qb: &mut QueryBuilder<MySql>, filters: &StatsQuery, tournament_column: &str) {
    if let Some(from) = filters.from {
        qb.push(" AND m.scheduled_at >= ").push_bind(from);
    }
    if let Some(to) = filters.to {
        qb.push(" AND m.scheduled_at <= ").push_bind(to);
    }
    if let Some(tournament_id) = filters.tournament_id {
        qb.push(format!(" AND {tournament_column} = "));
        qb.push_bind(tournament_id);
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SideStats {
    pub side: Side,
    /// Rounds included below — the denominator for every `avg_*_per_round` field.
    pub rounds_played: i64,
    pub rounds_won: i64,
    pub round_winrate: f64,
    pub total_kills: i64,
    pub avg_kills_per_round: f64,
    pub total_assists: i64,
    pub avg_assists_per_round: f64,
    /// Round score (`game_map_round_player_stats.score`) averaged per round —
    /// this is what ACS is derived from in-game, so it doubles as per-side ACS.
    pub total_score: i64,
    pub avg_acs_per_round: f64,
    /// Average damage per round, from `game_map_round_damages`.
    pub total_damage: i64,
    pub avg_adr_per_round: f64,
    /// Headshot rate across all recorded shots for these rounds.
    pub total_headshots: i64,
    pub total_bodyshots: i64,
    pub total_legshots: i64,
    pub headshot_percentage: f64,
    /// Rounds where this player/team landed the round's first kill.
    pub first_kills: i64,
    /// Rounds where this player/team suffered the round's first death.
    pub first_deaths: i64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SideAvg {
    pub atk: SideStats,
    pub def: SideStats,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AvgStatsResponse {
    /// Maps included in the average — the denominator for every `avg_*` field below.
    pub maps_played: i64,
    pub total_kills: i64,
    pub avg_kills: f64,
    pub total_deaths: i64,
    pub avg_deaths: f64,
    pub total_assists: i64,
    pub avg_assists: f64,
    pub total_acs: i64,
    pub avg_acs: f64,
    pub total_adr: i64,
    pub avg_adr: f64,
    pub total_kast_percentage: f64,
    pub avg_kast_percentage: f64,
    pub total_headshot_percentage: f64,
    pub avg_headshot_percentage: f64,
    pub total_first_kills: i64,
    pub avg_first_kills: f64,
    pub total_first_deaths: i64,
    pub avg_first_deaths: f64,
    pub kd_ratio: f64,
    pub by_side: SideAvg,
}

#[derive(Debug, FromRow)]
struct AvgStatsRow {
    maps_played: i64,
    total_kills: Option<i64>,
    avg_kills: Option<f64>,
    total_deaths: Option<i64>,
    avg_deaths: Option<f64>,
    total_assists: Option<i64>,
    avg_assists: Option<f64>,
    total_acs: Option<i64>,
    avg_acs: Option<f64>,
    total_adr: Option<i64>,
    avg_adr: Option<f64>,
    total_kast: Option<f64>,
    avg_kast: Option<f64>,
    total_hs: Option<f64>,
    avg_hs: Option<f64>,
    total_fk: Option<i64>,
    avg_fk: Option<f64>,
    total_fd: Option<i64>,
    avg_fd: Option<f64>,
}

pub async fn fetch_stats(
    db: &MySqlPool,
    kind: EntityKind,
    id: u64,
    filters: &StatsQuery,
) -> Result<Option<AvgStatsResponse>, sqlx::Error> {
    let exists_sql = match kind {
        EntityKind::Player => "SELECT EXISTS(SELECT 1 FROM players WHERE id = ?)",
        EntityKind::Team => "SELECT EXISTS(SELECT 1 FROM teams WHERE id = ?)",
    };
    let exists: bool = sqlx::query_scalar(exists_sql)
        .bind(id)
        .fetch_one(db)
        .await?;
    if !exists {
        return Ok(None);
    }

    let mut qb: QueryBuilder<MySql> = QueryBuilder::new(
        "SELECT
            COUNT(*) as maps_played,
            CAST(SUM(gps.kills) AS SIGNED) as total_kills,
            CAST(AVG(gps.kills) AS DOUBLE) as avg_kills,
            CAST(SUM(gps.deaths) AS SIGNED) as total_deaths,
            CAST(AVG(gps.deaths) AS DOUBLE) as avg_deaths,
            CAST(SUM(gps.assists) AS SIGNED) as total_assists,
            CAST(AVG(gps.assists) AS DOUBLE) as avg_assists,
            CAST(SUM(gps.acs) AS SIGNED) as total_acs,
            CAST(AVG(gps.acs) AS DOUBLE) as avg_acs,
            CAST(SUM(gps.adr) AS SIGNED) as total_adr,
            CAST(AVG(gps.adr) AS DOUBLE) as avg_adr,
            CAST(SUM(gps.kast_percentage) AS DOUBLE) as total_kast,
            CAST(AVG(gps.kast_percentage) AS DOUBLE) as avg_kast,
            CAST(SUM(gps.headshot_percentage) AS DOUBLE) as total_hs,
            CAST(AVG(gps.headshot_percentage) AS DOUBLE) as avg_hs,
            CAST(SUM(gps.first_kills) AS SIGNED) as total_fk,
            CAST(AVG(gps.first_kills) AS DOUBLE) as avg_fk,
            CAST(SUM(gps.first_deaths) AS SIGNED) as total_fd,
            CAST(AVG(gps.first_deaths) AS DOUBLE) as avg_fd
        FROM game_player_stats gps
        JOIN matches m ON gps.match_id = m.id
        WHERE "
    );
    qb.push(kind.gps_column()).push(" = ").push_bind(id);
    push_common_filters(&mut qb, filters, "gps.tournament_id");

    let row: AvgStatsRow = qb.build_query_as().fetch_one(db).await?;

    let avg_kills = row.avg_kills.unwrap_or(0.0);
    let avg_deaths = row.avg_deaths.unwrap_or(0.0);

    let atk = fetch_side_stats(db, kind, id, filters, Side::Atk).await?;
    let def = fetch_side_stats(db, kind, id, filters, Side::Def).await?;

    Ok(Some(AvgStatsResponse {
        maps_played: row.maps_played,
        total_kills: row.total_kills.unwrap_or(0),
        avg_kills,
        total_deaths: row.total_deaths.unwrap_or(0),
        avg_deaths,
        total_assists: row.total_assists.unwrap_or(0),
        avg_assists: row.avg_assists.unwrap_or(0.0),
        total_acs: row.total_acs.unwrap_or(0),
        avg_acs: row.avg_acs.unwrap_or(0.0),
        total_adr: row.total_adr.unwrap_or(0),
        avg_adr: row.avg_adr.unwrap_or(0.0),
        total_kast_percentage: row.total_kast.unwrap_or(0.0),
        avg_kast_percentage: row.avg_kast.unwrap_or(0.0),
        total_headshot_percentage: row.total_hs.unwrap_or(0.0),
        avg_headshot_percentage: row.avg_hs.unwrap_or(0.0),
        total_first_kills: row.total_fk.unwrap_or(0),
        avg_first_kills: row.avg_fk.unwrap_or(0.0),
        total_first_deaths: row.total_fd.unwrap_or(0),
        avg_first_deaths: row.avg_fd.unwrap_or(0.0),
        kd_ratio: if avg_deaths > 0.0 { avg_kills / avg_deaths } else { avg_kills },
        by_side: SideAvg { atk, def },
    }))
}

#[derive(Debug, FromRow)]
struct SideStatsRow {
    rounds_played: i64,
    rounds_won: Option<i64>,
    total_kills: Option<i64>,
    avg_kills_per_round: Option<f64>,
    total_assists: Option<i64>,
    avg_assists_per_round: Option<f64>,
    total_score: Option<i64>,
    avg_acs_per_round: Option<f64>,
    total_damage: Option<i64>,
    avg_adr_per_round: Option<f64>,
    total_headshots: Option<i64>,
    total_bodyshots: Option<i64>,
    total_legshots: Option<i64>,
}

#[derive(Debug, FromRow)]
struct FirstBloodRow {
    first_count: i64,
}

async fn fetch_side_stats(
    db: &MySqlPool,
    kind: EntityKind,
    id: u64,
    filters: &StatsQuery,
    side: Side,
) -> Result<SideStats, sqlx::Error> {
    let mut qb: QueryBuilder<MySql> = QueryBuilder::new(
        "SELECT
            COUNT(*) as rounds_played,
            CAST(SUM(CASE WHEN r.winning_team = gps.team_id THEN 1 ELSE 0 END) AS SIGNED) as rounds_won,
            CAST(SUM(ps.kills) AS SIGNED) as total_kills,
            CAST(AVG(ps.kills) AS DOUBLE) as avg_kills_per_round,
            CAST(SUM(ps.assists) AS SIGNED) as total_assists,
            CAST(AVG(ps.assists) AS DOUBLE) as avg_assists_per_round,
            CAST(SUM(ps.score) AS SIGNED) as total_score,
            CAST(AVG(ps.score) AS DOUBLE) as avg_acs_per_round,
            CAST(SUM(d.damage) AS SIGNED) as total_damage,
            CAST(AVG(COALESCE(d.damage, 0)) AS DOUBLE) as avg_adr_per_round,
            CAST(SUM(d.headshots) AS SIGNED) as total_headshots,
            CAST(SUM(d.bodyshots) AS SIGNED) as total_bodyshots,
            CAST(SUM(d.legshots) AS SIGNED) as total_legshots
        FROM game_map_round_player_stats ps
        JOIN game_map_rounds r ON ps.game_map_round_id = r.id
        JOIN game_player_stats gps
            ON gps.game_map_id = r.game_map_id AND gps.player_id = ps.player_id
        JOIN matches m ON r.match_id = m.id
        LEFT JOIN (
            SELECT game_map_round_id, attacker_player_id,
                SUM(damage) as damage, SUM(headshots) as headshots,
                SUM(bodyshots) as bodyshots, SUM(legshots) as legshots
            FROM game_map_round_damages
            GROUP BY game_map_round_id, attacker_player_id
        ) d ON d.game_map_round_id = ps.game_map_round_id AND d.attacker_player_id = ps.player_id
        WHERE "
    );
    qb.push(kind.gps_column()).push(" = ").push_bind(id);
    qb.push(" AND ").push(side.side_team_column()).push(" = gps.team_id");
    push_common_filters(&mut qb, filters, "r.tournament_id");

    let row: SideStatsRow = qb.build_query_as().fetch_one(db).await?;

    let rounds_played = row.rounds_played;
    let rounds_won = row.rounds_won.unwrap_or(0);
    let round_winrate = if rounds_played > 0 {
        rounds_won as f64 / rounds_played as f64
    } else {
        0.0
    };

    let total_headshots = row.total_headshots.unwrap_or(0);
    let total_bodyshots = row.total_bodyshots.unwrap_or(0);
    let total_legshots = row.total_legshots.unwrap_or(0);
    let total_shots = total_headshots + total_bodyshots + total_legshots;
    let headshot_percentage = if total_shots > 0 { total_headshots as f64 / total_shots as f64 } else { 0.0 };

    let first_kills = fetch_first_blood_count(db, kind, id, filters, side, FirstBloodEvent::Kill).await?;
    let first_deaths = fetch_first_blood_count(db, kind, id, filters, side, FirstBloodEvent::Death).await?;

    Ok(SideStats {
        side,
        rounds_played,
        rounds_won,
        round_winrate,
        total_kills: row.total_kills.unwrap_or(0),
        avg_kills_per_round: row.avg_kills_per_round.unwrap_or(0.0),
        total_assists: row.total_assists.unwrap_or(0),
        avg_assists_per_round: row.avg_assists_per_round.unwrap_or(0.0),
        total_score: row.total_score.unwrap_or(0),
        avg_acs_per_round: row.avg_acs_per_round.unwrap_or(0.0),
        total_damage: row.total_damage.unwrap_or(0),
        avg_adr_per_round: row.avg_adr_per_round.unwrap_or(0.0),
        total_headshots,
        total_bodyshots,
        total_legshots,
        headshot_percentage,
        first_kills,
        first_deaths,
    })
}

#[derive(Debug, Clone, Copy)]
enum FirstBloodEvent {
    Kill,
    Death,
}

impl FirstBloodEvent {
    fn anchor_column(self) -> &'static str {
        match self {
            FirstBloodEvent::Kill => "fk.kill_player_id",
            FirstBloodEvent::Death => "fk.victime_player_id",
        }
    }
}

/// Counts rounds where the round's earliest kill (`time_ms` ascending) was
/// landed by (`Kill`) or suffered by (`Death`) this player/team, restricted
/// to the requested side.
async fn fetch_first_blood_count(
    db: &MySqlPool,
    kind: EntityKind,
    id: u64,
    filters: &StatsQuery,
    side: Side,
    event: FirstBloodEvent,
) -> Result<i64, sqlx::Error> {
    let anchor_player_column = event.anchor_column();

    let mut qb: QueryBuilder<MySql> = QueryBuilder::new(format!(
        "SELECT COUNT(*) as first_count
        FROM (
            SELECT k.game_map_round_id, k.killer_player_id as kill_player_id, k.victim_player_id as victime_player_id,
                ROW_NUMBER() OVER (PARTITION BY k.game_map_round_id ORDER BY k.time_ms ASC) as rn
            FROM game_map_round_kills k
        ) fk
        JOIN game_map_rounds r ON fk.game_map_round_id = r.id
        JOIN game_player_stats gps
            ON gps.game_map_id = r.game_map_id AND gps.player_id = {anchor_player_column}
        JOIN matches m ON r.match_id = m.id
        WHERE fk.rn = 1 AND "
    ));
    qb.push(kind.gps_column()).push(" = ").push_bind(id);
    qb.push(" AND ").push(side.side_team_column()).push(" = gps.team_id");
    push_common_filters(&mut qb, filters, "r.tournament_id");

    let row: FirstBloodRow = qb.build_query_as().fetch_one(db).await?;
    Ok(row.first_count)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SituationEntry {
    pub played: i64,
    pub won: i64,
    pub winrate: f64,
}

#[derive(Debug, FromRow)]
struct RoundAliveRow {
    round_id: u64,
    winning_team: u64,
    atk_alive: Option<u8>,
    def_alive: Option<u8>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SideSituations {
    /// Keyed `"{team_alive}v{enemy_alive}"`, e.g. `"5v5"`, `"3v2"`, `"1v1"`.
    pub atk: std::collections::BTreeMap<String, SituationEntry>,
    pub def: std::collections::BTreeMap<String, SituationEntry>,
}

const MAX_ALIVE_PER_SIDE: i32 = 5;

pub async fn fetch_team_situations(
    db: &MySqlPool,
    team_id: u64,
    filters: &StatsQuery,
) -> Result<SideSituations, sqlx::Error> {
    let atk = fetch_situations_for_side(db, team_id, filters, Side::Atk).await?;
    let def = fetch_situations_for_side(db, team_id, filters, Side::Def).await?;
    Ok(SideSituations { atk, def })
}

async fn fetch_situations_for_side(
    db: &MySqlPool,
    team_id: u64,
    filters: &StatsQuery,
    side: Side,
) -> Result<std::collections::BTreeMap<String, SituationEntry>, sqlx::Error> {
    let mut qb: QueryBuilder<MySql> = QueryBuilder::new(
        "SELECT r.id as round_id, r.winning_team as winning_team,
            a.atk_alive as atk_alive, a.def_alive as def_alive
        FROM game_map_rounds r
        JOIN matches m ON r.match_id = m.id
        LEFT JOIN game_map_round_alive_states a ON a.game_map_round_id = r.id
        WHERE "
    );
    qb.push(side.side_team_column()).push(" = ").push_bind(team_id);
    push_round_filters(&mut qb, filters, "r.tournament_id");
    qb.push(" ORDER BY r.id, a.sequence ASC");

    let rows: Vec<RoundAliveRow> = qb.build_query_as().fetch_all(db).await?;

    let mut tally: HashMap<(i32, i32), (i64, i64)> = HashMap::new();
    for team_alive in 0..=MAX_ALIVE_PER_SIDE {
        for enemy_alive in 0..=MAX_ALIVE_PER_SIDE {
            tally.insert((team_alive, enemy_alive), (0, 0));
        }
    }

    let mut current_round: Option<u64> = None;
    let mut visited: std::collections::HashSet<(i32, i32)> = std::collections::HashSet::new();
    let mut round_won = false;

    fn flush(tally: &mut HashMap<(i32, i32), (i64, i64)>, visited: &std::collections::HashSet<(i32, i32)>, won: bool) {
        for &state in visited {
            let entry = tally.entry(state).or_insert((0, 0));
            entry.0 += 1;
            if won {
                entry.1 += 1;
            }
        }
    }

    for row in rows {
        if current_round != Some(row.round_id) {
            if current_round.is_some() {
                flush(&mut tally, &visited, round_won);
            }
            current_round = Some(row.round_id);
            visited.clear();
            round_won = row.winning_team == team_id;
        }
        // No snapshot for this round (ingestion gap): don't fabricate a state.
        let (Some(atk_alive), Some(def_alive)) = (row.atk_alive, row.def_alive) else {
            continue;
        };
        let (atk_alive, def_alive) = (atk_alive as i32, def_alive as i32);
        let (team_alive, enemy_alive) = match side {
            Side::Atk => (atk_alive, def_alive),
            Side::Def => (def_alive, atk_alive),
        };
        visited.insert((team_alive, enemy_alive));
    }
    if current_round.is_some() {
        flush(&mut tally, &visited, round_won);
    }

    Ok(tally.into_iter().map(|((team_alive, enemy_alive), (played, won))| {
        let winrate = if played > 0 { won as f64 / played as f64 } else { 0.0 };
        (format!("{team_alive}v{enemy_alive}"), SituationEntry { played, won, winrate })
    }).collect())
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PostPlantStats {
    pub played: i64,
    pub won: i64,
    pub winrate: f64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SidePostPlant {
    pub atk: PostPlantStats,
    pub def: PostPlantStats,
}

#[derive(Debug, FromRow)]
struct PostPlantRow {
    played: i64,
    won: Option<i64>,
}

/// Rounds played/won where a spike plant happened (`game_map_rounds.plant_site
/// IS NOT NULL`), split by side.
pub async fn fetch_team_post_plant(
    db: &MySqlPool,
    team_id: u64,
    filters: &StatsQuery,
) -> Result<SidePostPlant, sqlx::Error> {
    let atk = fetch_post_plant_for_side(db, team_id, filters, Side::Atk).await?;
    let def = fetch_post_plant_for_side(db, team_id, filters, Side::Def).await?;
    Ok(SidePostPlant { atk, def })
}

async fn fetch_post_plant_for_side(
    db: &MySqlPool,
    team_id: u64,
    filters: &StatsQuery,
    side: Side,
) -> Result<PostPlantStats, sqlx::Error> {
    let mut qb: QueryBuilder<MySql> = QueryBuilder::new(
        "SELECT
            COUNT(*) as played,
            CAST(SUM(CASE WHEN r.winning_team = "
    );
    qb.push_bind(team_id);
    qb.push(
        " THEN 1 ELSE 0 END) AS SIGNED) as won
        FROM game_map_rounds r
        JOIN matches m ON r.match_id = m.id
        WHERE r.plant_site IS NOT NULL AND "
    );
    qb.push(side.side_team_column()).push(" = ").push_bind(team_id);
    push_round_filters(&mut qb, filters, "r.tournament_id");

    let row: PostPlantRow = qb.build_query_as().fetch_one(db).await?;
    let won = row.won.unwrap_or(0);
    let winrate = if row.played > 0 { won as f64 / row.played as f64 } else { 0.0 };

    Ok(PostPlantStats { played: row.played, won, winrate })
}

/// Full team stats: the shared avg/side block plus team-only round context
/// (XvY situations and post-plant performance).
#[derive(Debug, Serialize, ToSchema)]
pub struct TeamStatsResponse {
    #[serde(flatten)]
    pub stats: AvgStatsResponse,
    pub situations: SideSituations,
    pub post_plant: SidePostPlant,
}

pub async fn fetch_team_stats(
    db: &MySqlPool,
    team_id: u64,
    filters: &StatsQuery,
) -> Result<Option<TeamStatsResponse>, sqlx::Error> {
    let Some(stats) = fetch_stats(db, EntityKind::Team, team_id, filters).await? else {
        return Ok(None);
    };
    let situations = fetch_team_situations(db, team_id, filters).await?;
    let post_plant = fetch_team_post_plant(db, team_id, filters).await?;

    Ok(Some(TeamStatsResponse { stats, situations, post_plant }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct SideWinrate {
    pub rounds_played: i64,
    pub rounds_won: i64,
    pub winrate: f64,
}

impl SideWinrate {
    fn from_counts(rounds_played: i64, rounds_won: i64) -> Self {
        let winrate = if rounds_played > 0 { rounds_won as f64 / rounds_played as f64 } else { 0.0 };
        Self { rounds_played, rounds_won, winrate }
    }
}

#[derive(Debug, Serialize, ToSchema)]
pub struct CompEntry {
    pub comp: Vec<String>,
    pub times_played: i64,
    pub wins: i64,
    pub losses: i64,
    pub winrate: f64,
    pub atk: SideWinrate,
    pub def: SideWinrate,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TeamMapEntry {
    pub map_name: String,
    pub times_played: i64,
    pub wins: i64,
    pub losses: i64,
    pub winrate: f64,
    pub atk: SideWinrate,
    pub def: SideWinrate,
    /// Comps played on this map, across all tournaments (pass `tournament_id`
    /// to `fetch_team_maps` to scope everything, including this, to one).
    pub comps: Vec<CompEntry>,
}

#[derive(Debug, FromRow)]
struct MapTotalsRow {
    map_name: String,
    times_played: i64,
    wins: i64,
}

#[derive(Debug, FromRow)]
struct MapSideRow {
    map_name: String,
    atk_rounds: i64,
    atk_wins: i64,
    def_rounds: i64,
    def_wins: i64,
}

#[derive(Debug, FromRow)]
struct CompRow {
    game_map_id: u64,
    map_name: String,
    agent_name: String,
    team_a_id: Option<u64>,
    team_b_id: Option<u64>,
    team_a_score: i32,
    team_b_score: i32,
}

#[derive(Debug, FromRow)]
struct RoundSideRow {
    game_map_id: u64,
    atk_team: Option<u64>,
    def_team: Option<u64>,
    winning_team: u64,
}

#[derive(Default, Clone, Copy)]
struct SideTally {
    atk_rounds: i64,
    atk_wins: i64,
    def_rounds: i64,
    def_wins: i64,
}

pub async fn fetch_team_maps(db: &MySqlPool, team_id: u64, tournament_id: Option<u64>) -> Result<Vec<TeamMapEntry>, sqlx::Error> {
    let mut map_totals_qb: QueryBuilder<MySql> = QueryBuilder::new(
        r#"
        SELECT
            gm.map_name as map_name,
            COUNT(*) as times_played,
            CAST(SUM(CASE
                WHEN m.team_a_id = "#
    );
    map_totals_qb.push_bind(team_id);
    map_totals_qb.push(" AND gm.team_a_score > gm.team_b_score THEN 1 WHEN m.team_b_id = ");
    map_totals_qb.push_bind(team_id);
    map_totals_qb.push(
        r#" AND gm.team_b_score > gm.team_a_score THEN 1 ELSE 0 END) AS SIGNED) as wins
        FROM game_maps gm
        JOIN matches m ON gm.match_id = m.id
        WHERE (m.team_a_id = "#
    );
    map_totals_qb.push_bind(team_id);
    map_totals_qb.push(" OR m.team_b_id = ");
    map_totals_qb.push_bind(team_id);
    map_totals_qb.push(") AND gm.is_completed = 1");
    if let Some(t_id) = tournament_id {
        map_totals_qb.push(" AND m.tournament_id = ").push_bind(t_id);
    }
    map_totals_qb.push(" GROUP BY gm.map_name");

    let map_totals: Vec<MapTotalsRow> = map_totals_qb.build_query_as().fetch_all(db).await?;

    let mut map_sides_qb: QueryBuilder<MySql> = QueryBuilder::new(
        r#"
        SELECT
            gm.map_name as map_name,
            CAST(SUM(CASE WHEN r.atk_team = "#
    );
    map_sides_qb.push_bind(team_id);
    map_sides_qb.push(" THEN 1 ELSE 0 END) AS SIGNED) as atk_rounds, CAST(SUM(CASE WHEN r.atk_team = ");
    map_sides_qb.push_bind(team_id);
    map_sides_qb.push(" AND r.winning_team = ");
    map_sides_qb.push_bind(team_id);
    map_sides_qb.push(" THEN 1 ELSE 0 END) AS SIGNED) as atk_wins, CAST(SUM(CASE WHEN r.def_team = ");
    map_sides_qb.push_bind(team_id);
    map_sides_qb.push(" THEN 1 ELSE 0 END) AS SIGNED) as def_rounds, CAST(SUM(CASE WHEN r.def_team = ");
    map_sides_qb.push_bind(team_id);
    map_sides_qb.push(" AND r.winning_team = ");
    map_sides_qb.push_bind(team_id);
    map_sides_qb.push(
        r#" THEN 1 ELSE 0 END) AS SIGNED) as def_wins
        FROM game_map_rounds r
        JOIN game_maps gm ON r.game_map_id = gm.id
        WHERE (r.atk_team = "#
    );
    map_sides_qb.push_bind(team_id);
    map_sides_qb.push(" OR r.def_team = ");
    map_sides_qb.push_bind(team_id);
    map_sides_qb.push(")");
    if let Some(t_id) = tournament_id {
        map_sides_qb.push(" AND r.tournament_id = ").push_bind(t_id);
    }
    map_sides_qb.push(" GROUP BY gm.map_name");

    let map_sides: Vec<MapSideRow> = map_sides_qb.build_query_as().fetch_all(db).await?;

    let mut comp_rows_qb: QueryBuilder<MySql> = QueryBuilder::new(
        r#"
        SELECT
            gps.game_map_id as game_map_id,
            gm.map_name as map_name,
            gps.agent_name as agent_name,
            m.team_a_id as team_a_id,
            m.team_b_id as team_b_id,
            gm.team_a_score as team_a_score,
            gm.team_b_score as team_b_score
        FROM game_player_stats gps
        JOIN game_maps gm ON gps.game_map_id = gm.id
        JOIN matches m ON gm.match_id = m.id
        WHERE gps.team_id = "#
    );
    comp_rows_qb.push_bind(team_id);
    comp_rows_qb.push(" AND gm.is_completed = 1");
    if let Some(t_id) = tournament_id {
        comp_rows_qb.push(" AND gps.tournament_id = ").push_bind(t_id);
    }
    comp_rows_qb.push(" ORDER BY gps.game_map_id");

    let comp_rows: Vec<CompRow> = comp_rows_qb.build_query_as().fetch_all(db).await?;

    let mut round_side_rows_qb: QueryBuilder<MySql> = QueryBuilder::new(
        r#"
        SELECT r.game_map_id as game_map_id, r.atk_team as atk_team, r.def_team as def_team, r.winning_team as winning_team
        FROM game_map_rounds r
        WHERE (r.atk_team = "#
    );
    round_side_rows_qb.push_bind(team_id);
    round_side_rows_qb.push(" OR r.def_team = ");
    round_side_rows_qb.push_bind(team_id);
    round_side_rows_qb.push(")");
    if let Some(t_id) = tournament_id {
        round_side_rows_qb.push(" AND r.tournament_id = ").push_bind(t_id);
    }

    let round_side_rows: Vec<RoundSideRow> = round_side_rows_qb.build_query_as().fetch_all(db).await?;

    let mut round_tally_by_map: HashMap<u64, SideTally> = HashMap::new();
    for row in &round_side_rows {
        let tally = round_tally_by_map.entry(row.game_map_id).or_default();
        if row.atk_team == Some(team_id) {
            tally.atk_rounds += 1;
            if row.winning_team == team_id {
                tally.atk_wins += 1;
            }
        }
        if row.def_team == Some(team_id) {
            tally.def_rounds += 1;
            if row.winning_team == team_id {
                tally.def_wins += 1;
            }
        }
    }

    struct MapInstance {
        map_name: String,
        agents: Vec<String>,
        won: bool,
    }

    let mut maps: HashMap<u64, MapInstance> = HashMap::new();
    for row in comp_rows {
        let entry = maps.entry(row.game_map_id).or_insert_with(|| {
            let won = (row.team_a_id == Some(team_id) && row.team_a_score > row.team_b_score)
                || (row.team_b_id == Some(team_id) && row.team_b_score > row.team_a_score);
            MapInstance {
                map_name: row.map_name.clone(),
                agents: Vec::new(),
                won,
            }
        });
        entry.agents.push(row.agent_name);
    }

    struct CompAcc {
        map_name: String,
        comp: Vec<String>,
        times_played: i64,
        wins: i64,
        losses: i64,
        side: SideTally,
    }

    // Comps are aggregated across all tournaments — grouped only by (map, comp).
    let mut comps: HashMap<(String, String), CompAcc> = HashMap::new();

    for (game_map_id, instance) in maps {
        let mut agents = instance.agents;
        agents.sort();
        let comp_key = agents.join(",");
        let key = (instance.map_name.clone(), comp_key);
        let round_tally = round_tally_by_map.get(&game_map_id).copied().unwrap_or_default();

        let acc = comps.entry(key).or_insert_with(|| CompAcc {
            map_name: instance.map_name,
            comp: agents.clone(),
            times_played: 0,
            wins: 0,
            losses: 0,
            side: SideTally::default(),
        });

        acc.times_played += 1;
        if instance.won {
            acc.wins += 1;
        } else {
            acc.losses += 1;
        }
        acc.side.atk_rounds += round_tally.atk_rounds;
        acc.side.atk_wins += round_tally.atk_wins;
        acc.side.def_rounds += round_tally.def_rounds;
        acc.side.def_wins += round_tally.def_wins;
    }

    let mut comps_by_map: HashMap<String, Vec<CompEntry>> = HashMap::new();
    for acc in comps.into_values() {
        comps_by_map.entry(acc.map_name.clone()).or_default().push(CompEntry {
            comp: acc.comp,
            times_played: acc.times_played,
            wins: acc.wins,
            losses: acc.losses,
            winrate: if acc.times_played > 0 { acc.wins as f64 / acc.times_played as f64 } else { 0.0 },
            atk: SideWinrate::from_counts(acc.side.atk_rounds, acc.side.atk_wins),
            def: SideWinrate::from_counts(acc.side.def_rounds, acc.side.def_wins),
        });
    }

    let map_side_by_name: HashMap<String, MapSideRow> = map_sides.into_iter().map(|r| (r.map_name.clone(), r)).collect();

    let mut result: Vec<TeamMapEntry> = map_totals.into_iter().map(|totals| {
        let losses = totals.times_played - totals.wins;
        let winrate = if totals.times_played > 0 { totals.wins as f64 / totals.times_played as f64 } else { 0.0 };

        let (atk, def) = map_side_by_name.get(&totals.map_name)
            .map(|s| (
                SideWinrate::from_counts(s.atk_rounds, s.atk_wins),
                SideWinrate::from_counts(s.def_rounds, s.def_wins),
            ))
            .unwrap_or((SideWinrate::from_counts(0, 0), SideWinrate::from_counts(0, 0)));

        let mut comps = comps_by_map.remove(&totals.map_name).unwrap_or_default();
        comps.sort_by_key(|c| std::cmp::Reverse(c.times_played));

        TeamMapEntry {
            map_name: totals.map_name,
            times_played: totals.times_played,
            wins: totals.wins,
            losses,
            winrate,
            atk,
            def,
            comps,
        }
    }).collect();

    result.sort_by_key(|r| std::cmp::Reverse(r.times_played));

    Ok(result)
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AgentStatsEntry {
    pub agent_name: String,
    /// Maps included below — the denominator for every `avg_*` field and pickrate.
    pub maps_played: i64,
    pub pickrate: f64,
    pub total_kills: i64,
    pub avg_kills: f64,
    pub total_deaths: i64,
    pub avg_deaths: f64,
    pub total_assists: i64,
    pub avg_assists: f64,
    pub total_acs: i64,
    pub avg_acs: f64,
    pub total_adr: i64,
    pub avg_adr: f64,
    pub total_kast_percentage: f64,
    pub avg_kast_percentage: f64,
    pub total_headshot_percentage: f64,
    pub avg_headshot_percentage: f64,
    pub total_first_kills: i64,
    pub avg_first_kills: f64,
    pub total_first_deaths: i64,
    pub avg_first_deaths: f64,
    pub kd_ratio: f64,
}

#[derive(Debug, FromRow)]
struct AgentStatsRow {
    agent_name: String,
    maps_played: i64,
    total_kills: Option<i64>,
    avg_kills: Option<f64>,
    total_deaths: Option<i64>,
    avg_deaths: Option<f64>,
    total_assists: Option<i64>,
    avg_assists: Option<f64>,
    total_acs: Option<i64>,
    avg_acs: Option<f64>,
    total_adr: Option<i64>,
    avg_adr: Option<f64>,
    total_kast: Option<f64>,
    avg_kast: Option<f64>,
    total_hs: Option<f64>,
    avg_hs: Option<f64>,
    total_fk: Option<i64>,
    avg_fk: Option<f64>,
    total_fd: Option<i64>,
    avg_fd: Option<f64>,
}

/// Per-agent stats and pickrate for a player across all recorded maps.
pub async fn fetch_player_agent_stats(db: &MySqlPool, player_id: u64) -> Result<Vec<AgentStatsEntry>, sqlx::Error> {
    let rows: Vec<AgentStatsRow> = sqlx::query_as(
        "SELECT
            agent_name,
            COUNT(*) as maps_played,
            CAST(SUM(kills) AS SIGNED) as total_kills,
            CAST(AVG(kills) AS DOUBLE) as avg_kills,
            CAST(SUM(deaths) AS SIGNED) as total_deaths,
            CAST(AVG(deaths) AS DOUBLE) as avg_deaths,
            CAST(SUM(assists) AS SIGNED) as total_assists,
            CAST(AVG(assists) AS DOUBLE) as avg_assists,
            CAST(SUM(acs) AS SIGNED) as total_acs,
            CAST(AVG(acs) AS DOUBLE) as avg_acs,
            CAST(SUM(adr) AS SIGNED) as total_adr,
            CAST(AVG(adr) AS DOUBLE) as avg_adr,
            CAST(SUM(kast_percentage) AS DOUBLE) as total_kast,
            CAST(AVG(kast_percentage) AS DOUBLE) as avg_kast,
            CAST(SUM(headshot_percentage) AS DOUBLE) as total_hs,
            CAST(AVG(headshot_percentage) AS DOUBLE) as avg_hs,
            CAST(SUM(first_kills) AS SIGNED) as total_fk,
            CAST(AVG(first_kills) AS DOUBLE) as avg_fk,
            CAST(SUM(first_deaths) AS SIGNED) as total_fd,
            CAST(AVG(first_deaths) AS DOUBLE) as avg_fd
         FROM game_player_stats
         WHERE player_id = ?
         GROUP BY agent_name
         ORDER BY maps_played DESC"
    )
        .bind(player_id)
        .fetch_all(db)
        .await?;

    let total: i64 = rows.iter().map(|r| r.maps_played).sum();

    Ok(rows.into_iter().map(|r| {
        let pickrate = if total > 0 { r.maps_played as f64 / total as f64 } else { 0.0 };
        let avg_kills = r.avg_kills.unwrap_or(0.0);
        let avg_deaths = r.avg_deaths.unwrap_or(0.0);

        AgentStatsEntry {
            agent_name: r.agent_name,
            maps_played: r.maps_played,
            pickrate,
            total_kills: r.total_kills.unwrap_or(0),
            avg_kills,
            total_deaths: r.total_deaths.unwrap_or(0),
            avg_deaths,
            total_assists: r.total_assists.unwrap_or(0),
            avg_assists: r.avg_assists.unwrap_or(0.0),
            total_acs: r.total_acs.unwrap_or(0),
            avg_acs: r.avg_acs.unwrap_or(0.0),
            total_adr: r.total_adr.unwrap_or(0),
            avg_adr: r.avg_adr.unwrap_or(0.0),
            total_kast_percentage: r.total_kast.unwrap_or(0.0),
            avg_kast_percentage: r.avg_kast.unwrap_or(0.0),
            total_headshot_percentage: r.total_hs.unwrap_or(0.0),
            avg_headshot_percentage: r.avg_hs.unwrap_or(0.0),
            total_first_kills: r.total_fk.unwrap_or(0),
            avg_first_kills: r.avg_fk.unwrap_or(0.0),
            total_first_deaths: r.total_fd.unwrap_or(0),
            avg_first_deaths: r.avg_fd.unwrap_or(0.0),
            kd_ratio: if avg_deaths > 0.0 { avg_kills / avg_deaths } else { avg_kills },
        }
    }).collect())
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WeaponStatsEntry {
    /// Weapon name — this includes ability "weapons" (e.g. Sova's Hunter's
    /// Fury/"Headhunter", Neon's "Overdrive") since `game_map_round_kills`
    /// lists ability kills the same way. That's expected, not a bug.
    pub weapon: String,
    /// Rounds where this was recorded as the held `weapon_id`. `null` — not
    /// `0` — when this weapon never appears in that column at all (e.g. an
    /// ability, or a weapon only ever picked up off the ground mid-round):
    /// a kill with a weapon doesn't prove it was bought/held for the round,
    /// so we don't fabricate a "played" count we don't actually have.
    pub times_played: Option<i64>,
    pub kills: i64,
}

#[derive(Debug, FromRow)]
struct WeaponPlayedRow {
    weapon: String,
    times_played: i64,
}

#[derive(Debug, FromRow)]
struct WeaponKillRow {
    weapon: String,
    kills: i64,
}

/// Weapon usage for a player or team: how many rounds it was held
/// (`game_map_round_player_stats.weapon_id`) and how many kills were landed
/// with it (`game_map_round_kills.weapon`). These two counts come from
/// different data sources and are merged by weapon name — see `times_played`.
pub async fn fetch_weapon_stats(db: &MySqlPool, kind: EntityKind, id: u64) -> Result<Vec<WeaponStatsEntry>, sqlx::Error> {
    let played_rows: Vec<WeaponPlayedRow> = match kind {
        EntityKind::Player => sqlx::query_as(
            "SELECT weapon_id as weapon, COUNT(*) as times_played
             FROM game_map_round_player_stats
             WHERE player_id = ? AND weapon_id IS NOT NULL
             GROUP BY weapon_id"
        )
            .bind(id)
            .fetch_all(db)
            .await?,
        EntityKind::Team => sqlx::query_as(
            "SELECT ps.weapon_id as weapon, COUNT(*) as times_played
             FROM game_map_round_player_stats ps
             JOIN game_map_rounds r ON ps.game_map_round_id = r.id
             JOIN game_player_stats gps ON gps.game_map_id = r.game_map_id AND gps.player_id = ps.player_id
             WHERE gps.team_id = ? AND ps.weapon_id IS NOT NULL
             GROUP BY ps.weapon_id"
        )
            .bind(id)
            .fetch_all(db)
            .await?,
    };

    let kill_rows: Vec<WeaponKillRow> = match kind {
        EntityKind::Player => sqlx::query_as(
            "SELECT weapon, COUNT(*) as kills
             FROM game_map_round_kills
             WHERE killer_player_id = ? AND weapon IS NOT NULL
             GROUP BY weapon"
        )
            .bind(id)
            .fetch_all(db)
            .await?,
        EntityKind::Team => sqlx::query_as(
            "SELECT k.weapon as weapon, COUNT(*) as kills
             FROM game_map_round_kills k
             JOIN game_map_rounds r ON k.game_map_round_id = r.id
             JOIN game_player_stats gps ON gps.game_map_id = r.game_map_id AND gps.player_id = k.killer_player_id
             WHERE gps.team_id = ? AND k.weapon IS NOT NULL
             GROUP BY k.weapon"
        )
            .bind(id)
            .fetch_all(db)
            .await?,
    };

    let mut played_by_weapon: HashMap<String, i64> = played_rows.into_iter().map(|r| (r.weapon, r.times_played)).collect();
    let mut kills_by_weapon: HashMap<String, i64> = kill_rows.into_iter().map(|r| (r.weapon, r.kills)).collect();

    let mut weapons: std::collections::BTreeSet<String> = played_by_weapon.keys().cloned().collect();
    weapons.extend(kills_by_weapon.keys().cloned());

    Ok(weapons.into_iter().map(|weapon| {
        let times_played = played_by_weapon.remove(&weapon);
        let kills = kills_by_weapon.remove(&weapon).unwrap_or(0);
        WeaponStatsEntry { weapon, times_played, kills }
    }).collect())
}

#[derive(Serialize, ToSchema)]
pub struct MatchStatsResponse {
    #[serde(flatten)]
    pub match_info: Match,
    pub team_a: Option<TeamWithScore>,
    pub team_b: Option<TeamWithScore>,
    pub vetos: Vec<MatchVeto>,
    pub maps: Vec<MapStatsFull>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct MapStatsFull {
    #[serde(flatten)]
    pub map: GameMap,
    pub player_stats: Vec<GamePlayerStat>,
    pub rounds: Vec<RoundStatsFull>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoundStatsFull {
    pub round_number: i32,
    pub winning_team: u64,
    pub win_type: String,
    pub atk_team: Option<u64>,
    pub def_team: Option<u64>,
    pub plant_site: Option<String>,
    pub player_stats: Vec<RoundPlayerStatFull>,
    pub kills: Vec<RoundKillEvent>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoundPlayerStatFull {
    pub player_id: u64,
    pub kills: i32,
    pub assists: i32,
    pub score: i32,
    pub economy_spent: i32,
    pub economy_remaining: i32,
    pub weapon_id: Option<String>,
    pub armor: Option<String>,
    pub damage: i32,
    pub headshots: i32,
    pub bodyshots: i32,
    pub legshots: i32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RoundKillEvent {
    /// `NULL` when the kill has no attributed killer (e.g. environmental).
    pub kill_player_id: Option<u64>,
    pub victim_player_id: u64,
    pub time_ms: i32,
}

#[derive(Debug, FromRow)]
struct MatchInfoRow {
    id: u64,
    tournament_id: u64,
    phase_id: u64,
    round_number: i32,
    round_name: String,
    scheduled_at: chrono::NaiveDateTime,
    status: String,
    best_of: i32,
    patch: Option<String>,
    team_a_score: i32,
    team_b_score: i32,
    ta_id: Option<u64>,
    ta_name: Option<String>,
    ta_short_name: Option<String>,
    ta_country_code: Option<String>,
    ta_bio: Option<String>,
    ta_socials: Option<String>,
    ta_vlr_id: Option<i32>,
    ta_is_active: Option<i8>,
    tb_id: Option<u64>,
    tb_name: Option<String>,
    tb_short_name: Option<String>,
    tb_country_code: Option<String>,
    tb_bio: Option<String>,
    tb_socials: Option<String>,
    tb_vlr_id: Option<i32>,
    tb_is_active: Option<i8>,
}

#[derive(Debug, FromRow)]
struct VetoRow {
    team_id: u64,
    map_name: String,
    veto_type: String,
    order: i32,
}

#[derive(Debug, FromRow)]
struct MapRow {
    id: u64,
    match_id: u64,
    api_match_id: Option<String>,
    map_name: String,
    team_a_score: Option<i32>,
    team_b_score: Option<i32>,
    order: i32,
    is_completed: i8,
}

#[derive(Debug, FromRow)]
struct GamePlayerStatRow {
    id: u64,
    game_map_id: u64,
    player_id: Option<u64>,
    team_id: u64,
    agent_name: String,
    kills: i32,
    deaths: i32,
    assists: i32,
    acs: i32,
    adr: i32,
    first_kills: i32,
    first_deaths: i32,
    kast_percentage: f64,
    headshot_percentage: f64,
}

#[derive(Debug, FromRow)]
struct RoundRow {
    id: u64,
    game_map_id: u64,
    round_number: i32,
    winning_team: u64,
    win_type: String,
    atk_team: Option<u64>,
    def_team: Option<u64>,
    plant_site: Option<String>,
}

#[derive(Debug, FromRow)]
struct RoundPlayerStatRawRow {
    game_map_round_id: u64,
    player_id: u64,
    kills: i32,
    assists: i32,
    score: i32,
    economy_spent: i32,
    economy_remaining: i32,
    weapon_id: Option<String>,
    armor: Option<String>,
}

#[derive(Debug, FromRow)]
struct RoundDamageRow {
    game_map_round_id: u64,
    attacker_player_id: Option<u64>,
    damage: i64,
    headshots: i64,
    bodyshots: i64,
    legshots: i64,
}

#[derive(Debug, FromRow)]
struct RoundKillRow {
    game_map_round_id: u64,
    kill_player_id: Option<u64>,
    victime_player_id: u64,
    time_ms: i32,
}

pub async fn fetch_match_stats(db: &MySqlPool, match_id: u64) -> Result<Option<MatchStatsResponse>, sqlx::Error> {
    let info: Option<MatchInfoRow> = sqlx::query_as(
        r#"
        SELECT
            m.id as id, m.tournament_id as tournament_id, m.phase_id as phase_id,
            m.round_number as round_number, m.round_name as round_name,
            m.scheduled_at as scheduled_at, m.status as status, m.best_of as best_of, m.patch as patch,
            m.team_a_score as team_a_score, m.team_b_score as team_b_score,
            ta.id as ta_id, ta.name as ta_name, ta.short_name as ta_short_name,
            ta.country_code as ta_country_code, ta.bio as ta_bio,
            ta.socials as ta_socials, ta.vlr_id as ta_vlr_id, ta.is_active as ta_is_active,
            tb.id as tb_id, tb.name as tb_name, tb.short_name as tb_short_name,
            tb.country_code as tb_country_code, tb.bio as tb_bio,
            tb.socials as tb_socials, tb.vlr_id as tb_vlr_id, tb.is_active as tb_is_active
        FROM matches m
        LEFT JOIN teams ta ON m.team_a_id = ta.id
        LEFT JOIN teams tb ON m.team_b_id = tb.id
        WHERE m.id = ?
        "#
    )
        .bind(match_id)
        .fetch_optional(db)
        .await?;

    let Some(info) = info else { return Ok(None) };

    let veto_rows: Vec<VetoRow> = sqlx::query_as(
        r#"SELECT team_id, map_name, `type` as veto_type, `order` FROM match_vetos WHERE match_id = ? ORDER BY `order` ASC"#
    )
        .bind(match_id)
        .fetch_all(db)
        .await?;

    let map_rows: Vec<MapRow> = sqlx::query_as(
        r#"SELECT id, match_id, api_match_id, map_name, team_a_score, team_b_score, `order`, is_completed
           FROM game_maps WHERE match_id = ? ORDER BY `order` ASC"#
    )
        .bind(match_id)
        .fetch_all(db)
        .await?;

    let player_stat_rows: Vec<GamePlayerStatRow> = sqlx::query_as(
        r#"SELECT
            id, game_map_id, player_id, team_id, agent_name,
            kills, deaths, assists, acs, adr, first_kills, first_deaths,
            CAST(kast_percentage AS DOUBLE) as kast_percentage,
            CAST(headshot_percentage AS DOUBLE) as headshot_percentage
           FROM game_player_stats WHERE match_id = ?"#
    )
        .bind(match_id)
        .fetch_all(db)
        .await?;

    let round_rows: Vec<RoundRow> = sqlx::query_as(
        r#"SELECT id, game_map_id, round_number, winning_team, win_type, atk_team, def_team, plant_site
           FROM game_map_rounds WHERE match_id = ? ORDER BY round_number ASC"#
    )
        .bind(match_id)
        .fetch_all(db)
        .await?;

    let round_player_stat_rows: Vec<RoundPlayerStatRawRow> = sqlx::query_as(
        r#"SELECT game_map_round_id, player_id, kills, assists, score, economy_spent, economy_remaining, weapon_id, armor
           FROM game_map_round_player_stats WHERE match_id = ?"#
    )
        .bind(match_id)
        .fetch_all(db)
        .await?;

    // One row per (round, attacker), pre-summed across every receiver they hit
    // that round — `game_map_round_damages` has one row per attacker/receiver pair.
    let round_damage_rows: Vec<RoundDamageRow> = sqlx::query_as(
        r#"SELECT d.game_map_round_id as game_map_round_id, d.attacker_player_id as attacker_player_id,
                  CAST(SUM(d.damage) AS SIGNED) as damage,
                  CAST(SUM(d.headshots) AS SIGNED) as headshots,
                  CAST(SUM(d.bodyshots) AS SIGNED) as bodyshots,
                  CAST(SUM(d.legshots) AS SIGNED) as legshots
           FROM game_map_round_damages d
           JOIN game_map_rounds r ON d.game_map_round_id = r.id
           WHERE r.match_id = ?
           GROUP BY d.game_map_round_id, d.attacker_player_id"#
    )
        .bind(match_id)
        .fetch_all(db)
        .await?;

    let round_kill_rows: Vec<RoundKillRow> = sqlx::query_as(
        r#"SELECT k.game_map_round_id as game_map_round_id, k.killer_player_id as kill_player_id,
                  k.victim_player_id as victime_player_id, k.time_ms as time_ms
           FROM game_map_round_kills k
           JOIN game_map_rounds r ON k.game_map_round_id = r.id
           WHERE r.match_id = ?
           ORDER BY k.time_ms ASC"#
    )
        .bind(match_id)
        .fetch_all(db)
        .await?;

    // Damage with no attributed attacker (e.g. environmental) can't be matched
    // to any player's round stats, so it's excluded here rather than decode-panicking.
    let mut damages_by_round_player: HashMap<(u64, u64), &RoundDamageRow> = HashMap::new();
    for row in &round_damage_rows {
        if let Some(attacker_player_id) = row.attacker_player_id {
            damages_by_round_player.insert((row.game_map_round_id, attacker_player_id), row);
        }
    }

    let mut kills_by_round: HashMap<u64, Vec<RoundKillEvent>> = HashMap::new();
    for row in round_kill_rows {
        kills_by_round.entry(row.game_map_round_id).or_default().push(RoundKillEvent {
            kill_player_id: row.kill_player_id,
            victim_player_id: row.victime_player_id,
            time_ms: row.time_ms,
        });
    }

    let mut round_player_stats_by_round: HashMap<u64, Vec<RoundPlayerStatFull>> = HashMap::new();
    for row in &round_player_stat_rows {
        let damage = damages_by_round_player.get(&(row.game_map_round_id, row.player_id));
        round_player_stats_by_round.entry(row.game_map_round_id).or_default().push(RoundPlayerStatFull {
            player_id: row.player_id,
            kills: row.kills,
            assists: row.assists,
            score: row.score,
            economy_spent: row.economy_spent,
            economy_remaining: row.economy_remaining,
            weapon_id: row.weapon_id.clone(),
            armor: row.armor.clone(),
            damage: damage.map(|d| d.damage as i32).unwrap_or(0),
            headshots: damage.map(|d| d.headshots as i32).unwrap_or(0),
            bodyshots: damage.map(|d| d.bodyshots as i32).unwrap_or(0),
            legshots: damage.map(|d| d.legshots as i32).unwrap_or(0),
        });
    }

    let mut rounds_by_map: HashMap<u64, Vec<RoundStatsFull>> = HashMap::new();
    for row in round_rows {
        rounds_by_map.entry(row.game_map_id).or_default().push(RoundStatsFull {
            round_number: row.round_number,
            winning_team: row.winning_team,
            win_type: row.win_type,
            atk_team: row.atk_team,
            def_team: row.def_team,
            plant_site: row.plant_site,
            player_stats: round_player_stats_by_round.remove(&row.id).unwrap_or_default(),
            kills: kills_by_round.remove(&row.id).unwrap_or_default(),
        });
    }

    let mut player_stats_by_map: HashMap<u64, Vec<GamePlayerStat>> = HashMap::new();
    for row in player_stat_rows {
        player_stats_by_map.entry(row.game_map_id).or_default().push(GamePlayerStat {
            id: row.id,
            player_id: row.player_id,
            team_id: row.team_id,
            agent_name: row.agent_name,
            kills: row.kills,
            deaths: row.deaths,
            assists: row.assists,
            acs: row.acs,
            adr: row.adr,
            first_kills: row.first_kills,
            first_deaths: row.first_deaths,
            kast_percentage: row.kast_percentage,
            headshot_percentage: row.headshot_percentage,
        });
    }

    let maps = map_rows.into_iter().map(|row| {
        let map_id = row.id;
        MapStatsFull {
            map: GameMap {
                id: row.id,
                match_id: row.match_id,
                api_match_id: row.api_match_id,
                map_name: row.map_name,
                team_a_score: row.team_a_score,
                team_b_score: row.team_b_score,
                order: row.order,
                is_completed: row.is_completed != 0,
            },
            player_stats: player_stats_by_map.remove(&map_id).unwrap_or_default(),
            rounds: rounds_by_map.remove(&map_id).unwrap_or_default(),
        }
    }).collect();

    let match_info = Match {
        id: info.id,
        tournament_id: info.tournament_id,
        phase_id: info.phase_id,
        round_number: info.round_number,
        round_name: info.round_name,
        scheduled_at: info.scheduled_at,
        status: info.status,
        best_of: info.best_of,
        patch: info.patch,
    };

    let team_a = info.ta_id.map(|team_id| TeamWithScore {
        team: Team::from_joined_row(
            team_id, info.ta_name, info.ta_short_name, info.ta_country_code,
            info.ta_socials.as_deref(), info.ta_bio, info.ta_vlr_id, info.ta_is_active,
        ),
        score: Some(info.team_a_score),
    });

    let team_b = info.tb_id.map(|team_id| TeamWithScore {
        team: Team::from_joined_row(
            team_id, info.tb_name, info.tb_short_name, info.tb_country_code,
            info.tb_socials.as_deref(), info.tb_bio, info.tb_vlr_id, info.tb_is_active,
        ),
        score: Some(info.team_b_score),
    });

    let vetos = veto_rows.into_iter().map(|row| MatchVeto {
        match_id,
        team_id: row.team_id,
        map_name: row.map_name,
        r#type: row.veto_type,
        order: row.order,
    }).collect();

    Ok(Some(MatchStatsResponse { match_info, team_a, team_b, vetos, maps }))
}
