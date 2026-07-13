use std::sync::Arc;
use std::time::Duration;
use redis::{AsyncCommands, Script};
use sqlx::QueryBuilder;
use sqlx::mysql::MySql;
use crate::AppState;
use crate::middleware::logging::{LogEntry, LOG_BUFFER_KEY};

const FLUSH_INTERVAL: Duration = Duration::from_secs(5 * 60);
const CHUNK_SIZE: usize = 500;

// Entries being written to MariaDB live here. If the process crashes mid-flush, this key
// survives in Redis and is re-processed on the next startup before claiming new entries.
const INFLIGHT_KEY: &str = "apilog:inflight";

// Atomically renames the live buffer to the inflight key so new writes go to a fresh key.
// Does nothing if the source is empty or the inflight key already exists (crash recovery pending).
const CLAIM_BUFFER_SCRIPT: &str = r#"
    if redis.call('EXISTS', KEYS[1]) == 0 then return 0 end
    if redis.call('EXISTS', KEYS[2]) ~= 0 then return 0 end
    redis.call('RENAME', KEYS[1], KEYS[2])
    return 1
"#;

pub async fn run(state: Arc<AppState>) {
    // Recover any inflight entries left over from a previous crash before entering the loop.
    flush_once(&state).await;

    loop {
        tokio::time::sleep(FLUSH_INTERVAL).await;
        flush_once(&state).await;
    }
}

async fn flush_once(state: &Arc<AppState>) {
    let mut redis = state.redis_local.clone();

    // Process any entries left in the inflight key from a previous crash.
    drain_inflight(&mut redis, state).await;

    // Atomically claim the live buffer. Skipped if inflight still exists (drain above failed),
    // to avoid overwriting unprocessed crash-recovery data.
    let claimed: i64 = match Script::new(CLAIM_BUFFER_SCRIPT)
        .key(LOG_BUFFER_KEY)
        .key(INFLIGHT_KEY)
        .invoke_async(&mut redis)
        .await
    {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!("Failed to claim log buffer from local Redis: {:?}", err);
            return;
        }
    };

    if claimed == 1 {
        drain_inflight(&mut redis, state).await;
    }
}

// Reads all entries from INFLIGHT_KEY, inserts them to MariaDB in chunks, and on full success
// deletes INFLIGHT_KEY. On any insert failure the key is left intact for the next flush cycle
// (at-least-once delivery: a crash after partial success may produce duplicate log rows).
async fn drain_inflight(redis: &mut redis::aio::ConnectionManager, state: &Arc<AppState>) {
    let raw_entries: Vec<String> = match redis.lrange(INFLIGHT_KEY, 0isize, -1isize).await {
        Ok(entries) => entries,
        Err(err) => {
            tracing::warn!("Failed to read inflight log buffer from local Redis: {:?}", err);
            return;
        }
    };

    if raw_entries.is_empty() {
        return;
    }

    let mut entries = Vec::with_capacity(raw_entries.len());
    for raw in &raw_entries {
        match serde_json::from_str::<LogEntry>(raw) {
            Ok(entry) => entries.push(entry),
            Err(err) => tracing::warn!("Dropping malformed request log entry: {:?}", err),
        }
    }

    if entries.is_empty() {
        let _: Result<i64, _> = redis.del(INFLIGHT_KEY).await;
        return;
    }

    for chunk in entries.chunks(CHUNK_SIZE) {
        if let Err(err) = insert_chunk(state, chunk).await {
            tracing::warn!(
                "Failed to flush {} request log entries to MariaDB (will retry next cycle): {:?}",
                chunk.len(),
                err
            );
            return; // Leave INFLIGHT_KEY intact; retry on next flush_once call.
        }
    }

    if let Err(err) = redis.del::<_, i64>(INFLIGHT_KEY).await {
        tracing::warn!(
            "Failed to delete inflight key after successful flush (entries duplicated on next retry): {:?}",
            err
        );
    }
}

async fn insert_chunk(state: &Arc<AppState>, chunk: &[crate::middleware::logging::LogEntry]) -> Result<(), sqlx::Error> {
    let mut builder: QueryBuilder<MySql> = QueryBuilder::new(
        "INSERT INTO api_request_log (api_key_id, method, endpoint, status_code, duration_ms, user_agent, created_at) "
    );

    builder.push_values(chunk, |mut b, entry| {
        b.push_bind(entry.api_key_id)
            .push_bind(&entry.method)
            .push_bind(&entry.endpoint)
            .push_bind(entry.status_code)
            .push_bind(entry.duration_ms)
            .push_bind(&entry.user_agent)
            .push_bind(entry.created_at.naive_utc());
    });

    builder.build().execute(&state.db).await?;
    Ok(())
}
