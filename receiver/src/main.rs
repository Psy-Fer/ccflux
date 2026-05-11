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

use model::UsagePayload;

/// 64 KB is far more than any legitimate ccflux payload (typical < 1 KB).
const BODY_LIMIT: usize = 64 * 1024;

/// Legitimate use: one request per CC turn, maybe 2–3 turns/minute in an intense session.
/// 30/minute per token gives comfortable headroom while blocking abuse.
const RATE_LIMIT_PER_MINUTE: u32 = 30;

#[derive(Clone)]
struct AppState {
    pool: Arc<SqlitePool>,
    rate_limiter: Arc<RateLimiter>,
}

#[tokio::main]
async fn main() {
    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "ccflux.db".to_string());
    let pool = db::init(&db_path).await.expect("failed to init database");

    let addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let state = AppState {
        pool: Arc::new(pool),
        rate_limiter: Arc::new(RateLimiter::new(RATE_LIMIT_PER_MINUTE)),
    };

    let app = Router::new()
        .route("/report", post(handle_report))
        .layer(DefaultBodyLimit::max(BODY_LIMIT))
        .with_state(state);

    println!("ccflux-receiver listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn handle_report(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<UsagePayload>,
) -> StatusCode {
    let token = match extract_bearer(&headers) {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED,
    };

    if !state.rate_limiter.allow(&token).await {
        return StatusCode::TOO_MANY_REQUESTS;
    }

    match db::insert_usage(&state.pool, &token, &payload).await {
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

/// Simple sliding-window rate limiter keyed by bearer token.
/// Memory is bounded by the number of distinct tokens seen; for an org deployment
/// with hundreds of users this is negligible. Old entries are evicted on access.
struct RateLimiter {
    // token → (request count in current window, window start)
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
