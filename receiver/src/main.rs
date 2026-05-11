use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    body::Bytes,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::post,
    Router,
};
use sqlx::SqlitePool;
use tokio::sync::Mutex;

mod db;
mod keys;
mod model;
mod token;

use db::SigVerifyResult;

#[derive(Clone)]
struct AppState {
    pool: Arc<SqlitePool>,
    rate_limiter: Arc<RateLimiter>,
    access_token_expiry_secs: u64,
    refresh_token_rolling_days: i64,
    require_signatures: bool,
}

#[tokio::main]
async fn main() {
    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "ccflux.db".to_string());

    let listen_addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let access_token_expiry_secs: u64 = env_or("ACCESS_TOKEN_EXPIRY_SECS", 28800);
    let refresh_token_rolling_days: i64 = env_or("REFRESH_TOKEN_ROLLING_DAYS", 90);
    let rate_limit_per_minute: u32 = env_or("RATE_LIMIT_PER_MINUTE", 30);
    let body_limit_kb: usize = env_or("BODY_LIMIT_KB", 64);
    let require_signatures: bool = std::env::var("REQUIRE_SIGNATURES")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);

    println!("ccflux-receiver config:");
    println!("  DATABASE_PATH              = {db_path}");
    println!("  LISTEN_ADDR                = {listen_addr}");
    println!("  ACCESS_TOKEN_EXPIRY_SECS   = {access_token_expiry_secs}");
    println!("  REFRESH_TOKEN_ROLLING_DAYS = {refresh_token_rolling_days}");
    println!("  RATE_LIMIT_PER_MINUTE      = {rate_limit_per_minute}");
    println!("  BODY_LIMIT_KB              = {body_limit_kb}");
    println!("  REQUIRE_SIGNATURES         = {require_signatures}");

    let pool = db::init(&db_path).await.expect("failed to init database");
    let pool = Arc::new(pool);

    let state = AppState {
        pool: pool.clone(),
        rate_limiter: Arc::new(RateLimiter::new(rate_limit_per_minute)),
        access_token_expiry_secs,
        refresh_token_rolling_days,
        require_signatures,
    };

    // Purge expired access tokens once per hour.
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            match db::purge_expired_access_tokens(&pool).await {
                Ok(n) if n > 0 => println!("purged {n} expired access tokens"),
                Err(e) => eprintln!("purge error: {e}"),
                _ => {}
            }
        }
    });

    let app = Router::new()
        .route("/token", post(token::handle_token))
        .route("/report", post(handle_report))
        .route("/register-key", post(keys::handle_register_key))
        .layer(DefaultBodyLimit::max(body_limit_kb * 1024))
        .with_state(state);

    println!("ccflux-receiver listening on {listen_addr}");
    let listener = tokio::net::TcpListener::bind(&listen_addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handle_report(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let access_token = match extract_bearer(&headers) {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };

    if !state.rate_limiter.allow(&access_token).await {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    // Resolve email (needed for signature lookup) before verifying.
    let email = match db::email_from_access_token(&state.pool, &access_token).await {
        Ok(Some(e)) => e,
        Ok(None) => return StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => {
            eprintln!("db error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    };

    let sig_header = headers
        .get("x-ccflux-signature")
        .and_then(|v| v.to_str().ok());
    let ts_header = headers
        .get("x-ccflux-timestamp")
        .and_then(|v| v.to_str().ok());

    match db::verify_signature(&state.pool, &email, &body, sig_header, ts_header).await {
        Ok(SigVerifyResult::Valid) => {}
        Ok(SigVerifyResult::NotPresent) if !state.require_signatures => {}
        Ok(SigVerifyResult::NotPresent) => {
            return sig_error("signature-required");
        }
        Ok(SigVerifyResult::TimestampMissing) => {
            return sig_error("timestamp-missing");
        }
        Ok(SigVerifyResult::TimestampStale) => {
            return sig_error("timestamp-stale");
        }
        Ok(SigVerifyResult::MalformedHeader) => {
            return sig_error("signature-invalid");
        }
        Ok(SigVerifyResult::KeyNotRegistered) => {
            return sig_error("key-not-registered");
        }
        Ok(SigVerifyResult::KeyRevoked) => {
            return sig_error("key-revoked");
        }
        Ok(SigVerifyResult::Invalid) => {
            return sig_error("signature-invalid");
        }
        Err(e) => {
            eprintln!("signature verify error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR.into_response();
        }
    }

    let payload: model::UsagePayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(_) => return StatusCode::BAD_REQUEST.into_response(),
    };

    match db::insert_usage(&state.pool, &access_token, &payload).await {
        Ok(true) => StatusCode::OK.into_response(),
        Ok(false) => StatusCode::UNAUTHORIZED.into_response(),
        Err(e) => {
            eprintln!("db error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn sig_error(code: &'static str) -> Response {
    (StatusCode::FORBIDDEN, [("x-ccflux-error", code)]).into_response()
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let val = headers.get("authorization")?.to_str().ok()?;
    val.strip_prefix("Bearer ").map(|s| s.to_string())
}

fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

struct RateLimiter {
    windows: Mutex<HashMap<String, (u32, Instant)>>,
    max_per_minute: u32,
}

impl RateLimiter {
    fn new(max_per_minute: u32) -> Self {
        Self {
            windows: Mutex::new(HashMap::new()),
            max_per_minute,
        }
    }

    async fn allow(&self, token: &str) -> bool {
        let mut map = self.windows.lock().await;
        let now = Instant::now();
        let entry = map.entry(token.to_string()).or_insert((0, now));

        if now.duration_since(entry.1) >= Duration::from_secs(60) {
            *entry = (1, now);
            return true;
        }
        if entry.0 >= self.max_per_minute {
            return false;
        }
        entry.0 += 1;
        true
    }
}
