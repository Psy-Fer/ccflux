use sqlx::{Row, SqlitePool};

use crate::model::UsagePayload;

pub async fn init(path: &str) -> Result<SqlitePool, sqlx::Error> {
    let url = format!("sqlite://{path}?mode=rwc");
    let pool = SqlitePool::connect(&url).await?;
    sqlx::query(include_str!("../../schema.sql"))
        .execute(&pool)
        .await?;
    Ok(pool)
}

/// Resolves the email from the bearer token, then inserts one row per model.
/// Returns false if the token is unknown or revoked (caller returns 401).
pub async fn insert_usage(
    pool: &SqlitePool,
    token: &str,
    payload: &UsagePayload,
) -> Result<bool, sqlx::Error> {
    let row =
        sqlx::query("SELECT email FROM user_tokens WHERE token = ? AND revoked = 0")
            .bind(token)
            .fetch_optional(pool)
            .await?;

    let email = match row {
        Some(r) => r.get::<String, _>("email"),
        None => return Ok(false),
    };

    for (model, usage) in &payload.models {
        sqlx::query(
            "INSERT OR IGNORE INTO usage_events
             (user_email, user_token, session_id, turn_index, timestamp_utc,
              session_start_utc, model, input_tokens, output_tokens,
              cache_read_tokens, cache_write_tokens, plugin_version, schema_version)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&email)
        .bind(token)
        .bind(&payload.session_id)
        .bind(payload.turn_index as i64)
        .bind(&payload.timestamp_utc)
        .bind(&payload.session_start_utc)
        .bind(model)
        .bind(usage.input_tokens as i64)
        .bind(usage.output_tokens as i64)
        .bind(usage.cache_read_tokens as i64)
        .bind(usage.cache_write_tokens as i64)
        .bind(&payload.plugin_version)
        .bind(payload.schema_version as i64)
        .execute(pool)
        .await?;
    }

    Ok(true)
}
