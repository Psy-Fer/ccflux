use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{Duration, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;

use crate::model::{TokenResponse, UsagePayload};

pub enum SigVerifyResult {
    Valid,
    NotPresent,
    TimestampMissing,
    TimestampStale,
    MalformedHeader,
    KeyNotRegistered,
    KeyRevoked,
    Invalid,
}

/// Registers a device public key for a given email. Idempotent — re-registering
/// the same key updates last_seen_at only.
pub async fn register_device_key(
    pool: &SqlitePool,
    email: &str,
    public_key: &str,
    device_id: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO device_keys (public_key, email, device_id, last_seen_at)
         VALUES (?, ?, ?, datetime('now'))
         ON CONFLICT(public_key) DO UPDATE SET last_seen_at = datetime('now')",
    )
    .bind(public_key)
    .bind(email)
    .bind(device_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Verifies the Ed25519 signature on a /report request.
///
/// Signature header format: `ed25519 <sig_b64> <pubkey_b64>`
/// Signed message: `body_bytes ++ b'\n' ++ timestamp_header_value`
pub async fn verify_signature(
    pool: &SqlitePool,
    email: &str,
    body: &[u8],
    sig_header: Option<&str>,
    ts_header: Option<&str>,
) -> Result<SigVerifyResult, sqlx::Error> {
    let sig_header = match sig_header {
        Some(h) => h,
        None => return Ok(SigVerifyResult::NotPresent),
    };

    let ts_str = match ts_header {
        Some(t) => t,
        None => return Ok(SigVerifyResult::TimestampMissing),
    };

    // Check timestamp is within 5 minutes.
    let ts = match chrono::DateTime::parse_from_rfc3339(ts_str) {
        Ok(t) => t,
        Err(_) => return Ok(SigVerifyResult::MalformedHeader),
    };
    let age_secs = (Utc::now() - ts.with_timezone(&Utc)).num_seconds().abs();
    if age_secs > 300 {
        return Ok(SigVerifyResult::TimestampStale);
    }

    // Parse header: "ed25519 <sig_b64> <pubkey_b64>"
    let parts: Vec<&str> = sig_header.splitn(3, ' ').collect();
    if parts.len() != 3 || parts[0] != "ed25519" {
        return Ok(SigVerifyResult::MalformedHeader);
    }
    let sig_b64 = parts[1];
    let pubkey_b64 = parts[2];

    let sig_bytes = match STANDARD.decode(sig_b64) {
        Ok(b) => b,
        Err(_) => return Ok(SigVerifyResult::MalformedHeader),
    };
    let pubkey_bytes = match STANDARD.decode(pubkey_b64) {
        Ok(b) => b,
        Err(_) => return Ok(SigVerifyResult::MalformedHeader),
    };

    let sig_arr: [u8; 64] = match sig_bytes.try_into() {
        Ok(a) => a,
        Err(_) => return Ok(SigVerifyResult::MalformedHeader),
    };
    let pubkey_arr: [u8; 32] = match pubkey_bytes.try_into() {
        Ok(a) => a,
        Err(_) => return Ok(SigVerifyResult::MalformedHeader),
    };

    // Look up the key for this email in device_keys.
    let row = sqlx::query("SELECT revoked FROM device_keys WHERE public_key = ? AND email = ?")
        .bind(pubkey_b64)
        .bind(email)
        .fetch_optional(pool)
        .await?;

    match row {
        None => return Ok(SigVerifyResult::KeyNotRegistered),
        Some(r) => {
            if r.get::<i64, _>("revoked") != 0 {
                return Ok(SigVerifyResult::KeyRevoked);
            }
        }
    }

    // Verify: sign(body ++ '\n' ++ timestamp)
    let verifying_key = match VerifyingKey::from_bytes(&pubkey_arr) {
        Ok(k) => k,
        Err(_) => return Ok(SigVerifyResult::MalformedHeader),
    };
    let signature = Signature::from_bytes(&sig_arr);

    let mut msg = body.to_vec();
    msg.push(b'\n');
    msg.extend_from_slice(ts_str.as_bytes());

    match verifying_key.verify(&msg, &signature) {
        Ok(()) => {
            // Update last_seen_at.
            let _ = sqlx::query(
                "UPDATE device_keys SET last_seen_at = datetime('now') WHERE public_key = ?",
            )
            .bind(pubkey_b64)
            .execute(pool)
            .await;
            Ok(SigVerifyResult::Valid)
        }
        Err(_) => Ok(SigVerifyResult::Invalid),
    }
}

#[cfg(test)]
pub async fn init_test_pool() -> SqlitePool {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query(include_str!("../../schema.sql"))
        .execute(&pool)
        .await
        .unwrap();
    pool
}

/// Verifies the DB is reachable. Used by GET /health.
pub async fn ping(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    sqlx::query("SELECT 1").execute(pool).await?;
    Ok(())
}

/// Count of non-expired access tokens. Used by GET /metrics.
pub async fn count_active_access_tokens(pool: &SqlitePool) -> Result<i64, sqlx::Error> {
    let row =
        sqlx::query("SELECT COUNT(*) as n FROM access_tokens WHERE expires_at > datetime('now')")
            .fetch_one(pool)
            .await?;
    Ok(row.get::<i64, _>("n"))
}

/// Returns the email associated with a valid (non-expired) access token, or None.
pub async fn email_from_access_token(
    pool: &SqlitePool,
    access_token: &str,
) -> Result<Option<String>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT email FROM access_tokens WHERE token = ? AND expires_at > datetime('now')",
    )
    .bind(access_token)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|r| r.get::<String, _>("email")))
}

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
    let result = sqlx::query("DELETE FROM access_tokens WHERE expires_at < datetime('now')")
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

// ── Admin dashboard queries ────────────────────────────────────────────────

pub struct AdminOrgSummary {
    pub total_users: i64,
    pub total_sessions: i64,
    pub total_turns: i64,
    pub total_input: i64,
    pub total_output: i64,
    pub total_cache_read: i64,
    pub total_cache_write: i64,
}

pub struct AdminUserStat {
    pub email: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub sessions: i64,
    pub turns: i64,
    pub last_active: String,
}

pub struct AdminModelStat {
    pub model: String,
    pub unique_users: i64,
    pub turns: i64,
    pub total_input: i64,
    pub total_output: i64,
    pub total_cache_read: i64,
    pub total_cache_write: i64,
    pub cache_hit_pct: f64,
}

#[allow(dead_code)]
pub struct AdminDailyStat {
    pub day: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub turns: i64,
}

pub struct AdminRecentEvent {
    pub received_at: String,
    pub user_email: String,
    pub session_id: String,
    pub turn_index: i64,
    pub model: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
}

pub struct AdminDeviceKey {
    pub email: String,
    pub device_id: String,
    pub public_key: String,
    pub registered_at: String,
    pub last_seen_at: String,
    pub revoked: bool,
}

pub async fn admin_org_summary(pool: &SqlitePool) -> Result<AdminOrgSummary, sqlx::Error> {
    let r = sqlx::query(
        "SELECT COUNT(DISTINCT user_email) as u, COUNT(DISTINCT session_id) as s, COUNT(*) as t,
                COALESCE(SUM(input_tokens),0) as i, COALESCE(SUM(output_tokens),0) as o,
                COALESCE(SUM(cache_read_tokens),0) as cr, COALESCE(SUM(cache_write_tokens),0) as cw
         FROM usage_events",
    )
    .fetch_one(pool)
    .await?;
    Ok(AdminOrgSummary {
        total_users: r.get("u"),
        total_sessions: r.get("s"),
        total_turns: r.get("t"),
        total_input: r.get("i"),
        total_output: r.get("o"),
        total_cache_read: r.get("cr"),
        total_cache_write: r.get("cw"),
    })
}

pub async fn admin_user_stats(pool: &SqlitePool) -> Result<Vec<AdminUserStat>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT user_email,
                COALESCE(SUM(input_tokens),0) as i, COALESCE(SUM(output_tokens),0) as o,
                COALESCE(SUM(cache_read_tokens),0) as cr, COALESCE(SUM(cache_write_tokens),0) as cw,
                COUNT(DISTINCT session_id) as s, COUNT(*) as t,
                COALESCE(MAX(timestamp_utc),'') as la
         FROM usage_events
         WHERE timestamp_utc >= datetime('now','-30 days')
         GROUP BY user_email
         ORDER BY (SUM(input_tokens)+SUM(output_tokens)) DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| AdminUserStat {
            email: r.get("user_email"),
            input_tokens: r.get("i"),
            output_tokens: r.get("o"),
            cache_read_tokens: r.get("cr"),
            cache_write_tokens: r.get("cw"),
            sessions: r.get("s"),
            turns: r.get("t"),
            last_active: r.get("la"),
        })
        .collect())
}

pub async fn admin_model_stats(pool: &SqlitePool) -> Result<Vec<AdminModelStat>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT model, COUNT(DISTINCT user_email) as u, COUNT(*) as t,
                COALESCE(SUM(input_tokens),0) as i, COALESCE(SUM(output_tokens),0) as o,
                COALESCE(SUM(cache_read_tokens),0) as cr, COALESCE(SUM(cache_write_tokens),0) as cw,
                ROUND(100.0*SUM(cache_read_tokens)/
                    NULLIF(SUM(input_tokens+cache_read_tokens+cache_write_tokens),0),1) as chp
         FROM usage_events
         GROUP BY model
         ORDER BY SUM(output_tokens) DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| AdminModelStat {
            model: r.get("model"),
            unique_users: r.get("u"),
            turns: r.get("t"),
            total_input: r.get("i"),
            total_output: r.get("o"),
            total_cache_read: r.get("cr"),
            total_cache_write: r.get("cw"),
            cache_hit_pct: r.get::<Option<f64>, _>("chp").unwrap_or(0.0),
        })
        .collect())
}

pub async fn admin_daily_stats(pool: &SqlitePool) -> Result<Vec<AdminDailyStat>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT date(timestamp_utc) as day,
                COALESCE(SUM(input_tokens),0) as i, COALESCE(SUM(output_tokens),0) as o,
                COALESCE(SUM(cache_read_tokens),0) as cr, COUNT(*) as t
         FROM usage_events
         WHERE timestamp_utc >= datetime('now','-30 days')
         GROUP BY date(timestamp_utc)
         ORDER BY day ASC",
    )
    .fetch_all(pool)
    .await?;

    let mut map: std::collections::HashMap<String, AdminDailyStat> = rows
        .into_iter()
        .map(|r| {
            let day: String = r.get("day");
            let s = AdminDailyStat {
                day: day.clone(),
                input_tokens: r.get("i"),
                output_tokens: r.get("o"),
                cache_read_tokens: r.get("cr"),
                turns: r.get("t"),
            };
            (day, s)
        })
        .collect();

    let mut result = Vec::with_capacity(30);
    for days_ago in (0i64..30).rev() {
        let day = (Utc::now() - Duration::days(days_ago))
            .format("%Y-%m-%d")
            .to_string();
        result.push(map.remove(&day).unwrap_or(AdminDailyStat {
            day: day.clone(),
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            turns: 0,
        }));
    }
    Ok(result)
}

pub async fn admin_recent_events(pool: &SqlitePool) -> Result<Vec<AdminRecentEvent>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT received_at, user_email, session_id, turn_index, model,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens
         FROM usage_events
         ORDER BY received_at DESC
         LIMIT 50",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| AdminRecentEvent {
            received_at: r.get("received_at"),
            user_email: r.get("user_email"),
            session_id: r.get("session_id"),
            turn_index: r.get("turn_index"),
            model: r.get("model"),
            input_tokens: r.get("input_tokens"),
            output_tokens: r.get("output_tokens"),
            cache_read_tokens: r.get("cache_read_tokens"),
            cache_write_tokens: r.get("cache_write_tokens"),
        })
        .collect())
}

pub async fn admin_device_keys(pool: &SqlitePool) -> Result<Vec<AdminDeviceKey>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT email, COALESCE(device_id,'') as device_id, public_key,
                COALESCE(registered_at,'') as registered_at,
                COALESCE(last_seen_at,'') as last_seen_at, revoked
         FROM device_keys
         ORDER BY email, registered_at DESC",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| AdminDeviceKey {
            email: r.get("email"),
            device_id: r.get("device_id"),
            public_key: r.get("public_key"),
            registered_at: r.get("registered_at"),
            last_seen_at: r.get("last_seen_at"),
            revoked: r.get::<i64, _>("revoked") != 0,
        })
        .collect())
}

/// Fetches all usage events from the last 30 days for billing window computation.
/// The window algorithm runs in Rust (see `billing::compute_billing_windows`).
pub async fn fetch_events_for_windows(
    pool: &SqlitePool,
) -> Result<Vec<crate::billing::RawEvent>, sqlx::Error> {
    let rows = sqlx::query(
        "SELECT user_email, timestamp_utc, session_id,
                input_tokens, output_tokens, cache_read_tokens, cache_write_tokens
         FROM usage_events
         WHERE timestamp_utc >= datetime('now', '-30 days')
         ORDER BY user_email, timestamp_utc",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .iter()
        .map(|r| crate::billing::RawEvent {
            user_email: r.get("user_email"),
            timestamp_utc: r.get("timestamp_utc"),
            session_id: r.get("session_id"),
            input_tokens: r.get("input_tokens"),
            output_tokens: r.get("output_tokens"),
            cache_read_tokens: r.get("cache_read_tokens"),
            cache_write_tokens: r.get("cache_write_tokens"),
        })
        .collect())
}

pub async fn admin_revoke_key(pool: &SqlitePool, public_key: &str) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE device_keys SET revoked = 1 WHERE public_key = ?")
        .bind(public_key)
        .execute(pool)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn refresh_token_expiry_rolls_forward_on_use() {
        let pool = init_test_pool().await;

        // Insert a refresh token that expires in 1 day.
        sqlx::query(
            "INSERT INTO refresh_tokens (token, email, expires_at)
             VALUES ('rtok', 'u@example.org', datetime('now', '+1 day'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        let before: String =
            sqlx::query("SELECT expires_at FROM refresh_tokens WHERE token = 'rtok'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("expires_at");

        // Call /token with rolling_days = 90.
        let resp = issue_access_token(&pool, "rtok", 3600, 90).await.unwrap();
        assert!(resp.is_some(), "should issue an access token");

        let after: String =
            sqlx::query("SELECT expires_at FROM refresh_tokens WHERE token = 'rtok'")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("expires_at");

        // expires_at must have advanced beyond the original +1 day value.
        assert!(
            after > before,
            "expires_at should roll forward: before={before}, after={after}"
        );
    }

    #[tokio::test]
    async fn expired_refresh_token_returns_none() {
        let pool = init_test_pool().await;

        sqlx::query(
            "INSERT INTO refresh_tokens (token, email, expires_at)
             VALUES ('rtok_exp', 'u@example.org', datetime('now', '-1 second'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        let resp = issue_access_token(&pool, "rtok_exp", 3600, 90)
            .await
            .unwrap();
        assert!(resp.is_none(), "expired token should return None");
    }

    #[tokio::test]
    async fn revoked_refresh_token_returns_none() {
        let pool = init_test_pool().await;

        sqlx::query(
            "INSERT INTO refresh_tokens (token, email, revoked, expires_at)
             VALUES ('rtok_rev', 'u@example.org', 1, datetime('now', '+90 days'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        let resp = issue_access_token(&pool, "rtok_rev", 3600, 90)
            .await
            .unwrap();
        assert!(resp.is_none(), "revoked token should return None");
    }
}
