use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::{
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use sqlx::SqlitePool;
use tokio::sync::Mutex;

mod db;
mod model;
mod token;

use model::UsagePayload;

#[derive(Clone)]
struct AppState {
    pool: Arc<SqlitePool>,
    rate_limiter: Arc<RateLimiter>,
    access_token_expiry_secs: u64,
    refresh_token_rolling_days: i64,
}

#[tokio::main]
async fn main() {
    let db_path = std::env::var("DATABASE_PATH")
        .unwrap_or_else(|_| "ccflux.db".to_string());

    let listen_addr = std::env::var("LISTEN_ADDR")
        .unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    let access_token_expiry_secs: u64 = env_or("ACCESS_TOKEN_EXPIRY_SECS", 28800);
    let refresh_token_rolling_days: i64 = env_or("REFRESH_TOKEN_ROLLING_DAYS", 90);
    let rate_limit_per_minute: u32 = env_or("RATE_LIMIT_PER_MINUTE", 30);
    let body_limit_kb: usize = env_or("BODY_LIMIT_KB", 64);

    println!("ccflux-receiver config:");
    println!("  DATABASE_PATH              = {db_path}");
    println!("  LISTEN_ADDR                = {listen_addr}");
    println!("  ACCESS_TOKEN_EXPIRY_SECS   = {access_token_expiry_secs}");
    println!("  REFRESH_TOKEN_ROLLING_DAYS = {refresh_token_rolling_days}");
    println!("  RATE_LIMIT_PER_MINUTE      = {rate_limit_per_minute}");
    println!("  BODY_LIMIT_KB              = {body_limit_kb}");

    let pool = db::init(&db_path).await.expect("failed to init database");
    let pool = Arc::new(pool);

    let state = AppState {
        pool: pool.clone(),
        rate_limiter: Arc::new(RateLimiter::new(rate_limit_per_minute)),
        access_token_expiry_secs,
        refresh_token_rolling_days,
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
        .layer(DefaultBodyLimit::max(body_limit_kb * 1024))
        .with_state(state);

    println!("ccflux-receiver listening on {listen_addr}");
    let listener = tokio::net::TcpListener::bind(&listen_addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handle_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UsagePayload>,
) -> StatusCode {
    let access_token = match extract_bearer(&headers) {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED,
    };

    if !state.rate_limiter.allow(&access_token).await {
        return StatusCode::TOO_MANY_REQUESTS;
    }

    match db::insert_usage(&state.pool, &access_token, &payload).await {
        Ok(true) => StatusCode::OK,
        Ok(false) => StatusCode::UNAUTHORIZED,
        Err(e) => {
            eprintln!("db error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let val = headers.get("authorization")?.to_str().ok()?;
    val.strip_prefix("Bearer ").map(|s| s.to_string())
}

/// Reads an env var and parses it, falling back to `default` on missing or invalid values.
fn env_or<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Per-token sliding-window rate limiter.
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
