use chrono::{Duration, Utc};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::model::{TokenResponse, UsagePayload};

pub async fn init(path: &str) -> Result<SqlitePool, sqlx::Error> {
    let url = format!("sqlite://{path}?mode=rwc");
    let pool = SqlitePool::connect(&url).await?;
    sqlx::query(include_str!("../../schema.sql"))
        .execute(&pool)
        .await?;
    Ok(pool)
}

/// Validates a refresh token and returns a short-lived access token.
/// Reuses an existing valid token if one exists with >5 minutes remaining.
/// Returns None if the refresh token is unknown, revoked, or expired.
pub async fn issue_access_token(
    pool: &SqlitePool,
    refresh_token: &str,
    expiry_secs: u64,
    rolling_days: i64,
) -> Result<Option<TokenResponse>, sqlx::Error> {
    // Validate the refresh token.
    let row = sqlx::query(
        "SELECT email FROM refresh_tokens
         WHERE token = ? AND revoked = 0 AND expires_at > datetime('now')",
    )
    .bind(refresh_token)
    .fetch_optional(pool)
    .await?;

    let email = match row {
        Some(r) => r.get::<String, _>("email"),
        None => return Ok(None),
    };

    // Rolling expiry: every successful use pushes the refresh token forward.
    // Active users never see their token expire.
    sqlx::query(
        "UPDATE refresh_tokens SET expires_at = datetime('now', '+' || ? || ' days')
         WHERE token = ?",
    )
    .bind(rolling_days)
    .bind(refresh_token)
    .execute(pool)
    .await?;

    // Reuse an existing access token if it has more than 5 minutes left.
    let existing = sqlx::query(
        "SELECT token, expires_at FROM access_tokens
         WHERE refresh_token = ? AND expires_at > datetime('now', '+5 minutes')
         ORDER BY expires_at DESC LIMIT 1",
    )
    .bind(refresh_token)
    .fetch_optional(pool)
    .await?;

    if let Some(row) = existing {
        return Ok(Some(TokenResponse {
            access_token: row.get("token"),
            expires_at: row.get("expires_at"),
            token_type: "Bearer".to_string(),
        }));
    }

    // Issue a new access token.
    let token = Uuid::new_v4().to_string().replace('-', "");
    let expires_at = Utc::now() + Duration::seconds(expiry_secs as i64);
    let expires_at_str = expires_at.to_rfc3339();

    sqlx::query(
        "INSERT INTO access_tokens (token, refresh_token, email, expires_at)
         VALUES (?, ?, ?, ?)",
    )
    .bind(&token)
    .bind(refresh_token)
    .bind(&email)
    .bind(&expires_at_str)
    .execute(pool)
    .await?;

    Ok(Some(TokenResponse {
        access_token: token,
        expires_at: expires_at_str,
        token_type: "Bearer".to_string(),
    }))
}

/// Validates an access token and inserts one usage row per model.
/// Returns false if the token is unknown or expired (caller returns 401).
/// Stores the refresh_token in user_token for a stable audit trail across rotations.
pub async fn insert_usage(
    pool: &SqlitePool,
    access_token: &str,
    payload: &UsagePayload,
) -> Result<bool, sqlx::Error> {
    let row = sqlx::query(
        "SELECT email, refresh_token FROM access_tokens
         WHERE token = ? AND expires_at > datetime('now')",
    )
    .bind(access_token)
    .fetch_optional(pool)
    .await?;

    let (email, refresh_token) = match row {
        Some(r) => (
            r.get::<String, _>("email"),
            r.get::<String, _>("refresh_token"),
        ),
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
        .bind(&refresh_token)
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

/// Removes expired access tokens. Called periodically by a background task.
pub async fn purge_expired_access_tokens(pool: &SqlitePool) -> Result<u64, sqlx::Error> {
    let result =
        sqlx::query("DELETE FROM access_tokens WHERE expires_at < datetime('now')")
            .execute(pool)
            .await?;
    Ok(result.rows_affected())
}
