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
