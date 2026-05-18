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

mod admin;
mod billing;
mod db;
mod health;
mod keys;
mod model;
mod tiers;
mod token;

use db::SigVerifyResult;
use health::Metrics;

#[derive(Clone)]
struct AppState {
    pool: Arc<SqlitePool>,
    rate_limiter: Arc<RateLimiter>,
    metrics: Arc<Metrics>,
    access_token_expiry_secs: u64,
    refresh_token_rolling_days: i64,
    require_signatures: bool,
    admin_token: Option<String>,
    cookie_secure: bool,
    tier_cache: Arc<Mutex<HashMap<String, tiers::TierClassification>>>,
    base_url: String,
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
    let admin_token: Option<String> = std::env::var("ADMIN_TOKEN").ok().filter(|s| !s.is_empty());
    let base_url: String = std::env::var("BASE_URL").unwrap_or_default();
    let cookie_secure: bool = std::env::var("COOKIE_SECURE")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false);
    let tier_inference_days: i64 = env_or("TIER_INFERENCE_DAYS", 90);
    let tier_inference_interval_secs: u64 = env_or("TIER_INFERENCE_INTERVAL_SECS", 600);

    println!("ccflux-receiver config:");
    println!("  DATABASE_PATH              = {db_path}");
    println!("  LISTEN_ADDR                = {listen_addr}");
    println!("  ACCESS_TOKEN_EXPIRY_SECS   = {access_token_expiry_secs}");
    println!("  REFRESH_TOKEN_ROLLING_DAYS = {refresh_token_rolling_days}");
    println!("  RATE_LIMIT_PER_MINUTE      = {rate_limit_per_minute}");
    println!("  BODY_LIMIT_KB              = {body_limit_kb}");
    println!("  REQUIRE_SIGNATURES         = {require_signatures}");
    println!(
        "  ADMIN_TOKEN                = {}",
        if admin_token.is_some() {
            "set"
        } else {
            "unset (dashboard disabled)"
        }
    );
    println!(
        "  BASE_URL                   = {}",
        if base_url.is_empty() {
            "(unset)"
        } else {
            &base_url
        }
    );
    println!("  COOKIE_SECURE              = {cookie_secure}");
    println!("  TIER_INFERENCE_DAYS        = {tier_inference_days}");
    println!("  TIER_INFERENCE_INTERVAL    = {tier_inference_interval_secs}s");

    let pool = db::init(&db_path).await.expect("failed to init database");
    let pool = Arc::new(pool);

    // Seed the in-memory tier cache from persisted hints so the dashboard is
    // populated immediately on restart, before the first inference pass runs.
    let initial_tiers = db::load_tier_hints(&pool).await.unwrap_or_default();
    let tier_cache: Arc<Mutex<HashMap<String, tiers::TierClassification>>> =
        Arc::new(Mutex::new(initial_tiers));

    let state = AppState {
        pool: pool.clone(),
        rate_limiter: Arc::new(RateLimiter::new(rate_limit_per_minute)),
        metrics: Arc::new(Metrics::new()),
        access_token_expiry_secs,
        refresh_token_rolling_days,
        require_signatures,
        admin_token,
        base_url,
        cookie_secure,
        tier_cache: tier_cache.clone(),
    };

    // Purge expired access tokens once per hour.
    {
        let pool = pool.clone();
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
    }

    // Recompute tier classifications periodically.  First tick fires immediately
    // (tokio interval default), so classifications are fresh on startup.
    {
        let pool = pool.clone();
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(Duration::from_secs(tier_inference_interval_secs));
            loop {
                interval.tick().await;
                match run_tier_inference(&pool, tier_inference_days, &tier_cache).await {
                    Ok(n) => println!("tier inference: classified {n} users"),
                    Err(e) => eprintln!("tier inference error: {e}"),
                }
            }
        });
    }

    let app = Router::new()
        .merge(admin::router())
        .route("/token", post(token::handle_token))
        .route("/report", post(handle_report))
        .route("/register-key", post(keys::handle_register_key))
        .route("/health", axum::routing::get(health::handle_health))
        .route("/metrics", axum::routing::get(health::handle_metrics))
        .layer(DefaultBodyLimit::max(body_limit_kb * 1024))
        .with_state(state);

    println!("ccflux-receiver listening on {listen_addr}");
    let listener = tokio::net::TcpListener::bind(&listen_addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

async fn run_tier_inference(
    pool: &SqlitePool,
    lookback_days: i64,
    cache: &Arc<Mutex<HashMap<String, tiers::TierClassification>>>,
) -> Result<usize, String> {
    let events = db::fetch_events_for_tier_inference(pool, lookback_days)
        .await
        .map_err(|e| e.to_string())?;
    let windows = billing::compute_billing_windows(events);
    let new_tiers = tiers::infer_tiers(&windows);
    let n = new_tiers.len();
    db::save_tier_hints(pool, &new_tiers)
        .await
        .map_err(|e| e.to_string())?;
    *cache.lock().await = new_tiers;
    Ok(n)
}

async fn handle_report(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let access_token = match extract_bearer(&headers) {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED.into_response(),
    };

    if !state.rate_limiter.allow(&access_token).await {
        state.metrics.inc(&state.metrics.reports_rate_limited);
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }

    // Resolve email (needed for signature lookup) before verifying.
    let email = match db::email_from_access_token(&state.pool, &access_token).await {
        Ok(Some(e)) => e,
        Ok(None) => {
            state.metrics.inc(&state.metrics.reports_auth_rejected);
            return StatusCode::UNAUTHORIZED.into_response();
        }
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
            state.metrics.inc(&state.metrics.reports_sig_rejected);
            return sig_error("signature-required");
        }
        Ok(SigVerifyResult::TimestampMissing) => {
            state.metrics.inc(&state.metrics.reports_sig_rejected);
            return sig_error("timestamp-missing");
        }
        Ok(SigVerifyResult::TimestampStale) => {
            state.metrics.inc(&state.metrics.reports_sig_rejected);
            return sig_error("timestamp-stale");
        }
        Ok(SigVerifyResult::MalformedHeader) => {
            state.metrics.inc(&state.metrics.reports_sig_rejected);
            return sig_error("signature-invalid");
        }
        Ok(SigVerifyResult::KeyNotRegistered) => {
            state.metrics.inc(&state.metrics.reports_sig_rejected);
            return sig_error("key-not-registered");
        }
        Ok(SigVerifyResult::KeyRevoked) => {
            state.metrics.inc(&state.metrics.reports_sig_rejected);
            return sig_error("key-revoked");
        }
        Ok(SigVerifyResult::Invalid) => {
            state.metrics.inc(&state.metrics.reports_sig_rejected);
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

    if !is_valid_payload(&payload) {
        return StatusCode::BAD_REQUEST.into_response();
    }

    let device_id = if let Some(h) = sig_header {
        let parts: Vec<&str> = h.splitn(3, ' ').collect();
        if parts.len() == 3 {
            db::device_id_from_pubkey(&state.pool, parts[2])
                .await
                .unwrap_or_default()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    match db::insert_usage(&state.pool, &access_token, &device_id, &payload).await {
        Ok(true) => {
            state.metrics.inc(&state.metrics.reports_accepted);
            StatusCode::OK.into_response()
        }
        Ok(false) => {
            state.metrics.inc(&state.metrics.reports_auth_rejected);
            StatusCode::UNAUTHORIZED.into_response()
        }
        Err(e) => {
            eprintln!("db error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR.into_response()
        }
    }
}

fn is_valid_payload(p: &model::UsagePayload) -> bool {
    if p.session_id.is_empty() || p.session_id.len() > 64 {
        return false;
    }
    if p.timestamp_utc.len() > 64 {
        return false;
    }
    if p.session_start_utc.as_deref().is_some_and(|s| s.len() > 64) {
        return false;
    }
    if p.plugin_version.as_deref().is_some_and(|s| s.len() > 64) {
        return false;
    }
    if p.models.len() > 20 {
        return false;
    }
    if p.models.keys().any(|k| k.is_empty() || k.len() > 128) {
        return false;
    }
    true
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use base64::{engine::general_purpose::STANDARD, Engine};
    use ed25519_dalek::{Signer, SigningKey};
    use tower::util::ServiceExt;

    async fn test_state() -> (AppState, Arc<SqlitePool>) {
        let pool = Arc::new(db::init_test_pool().await);
        let state = AppState {
            pool: pool.clone(),
            rate_limiter: Arc::new(RateLimiter::new(100)),
            metrics: Arc::new(Metrics::new()),
            access_token_expiry_secs: 3600,
            refresh_token_rolling_days: 90,
            require_signatures: false,
            admin_token: None,
            base_url: String::new(),
            cookie_secure: false,
            tier_cache: Arc::new(Mutex::new(HashMap::new())),
        };
        (state, pool)
    }

    fn build_app(state: AppState) -> Router {
        Router::new()
            .route("/token", post(token::handle_token))
            .route("/report", post(handle_report))
            .route("/register-key", post(keys::handle_register_key))
            .route("/health", get(health::handle_health))
            .route("/metrics", get(health::handle_metrics))
            .with_state(state)
    }

    async fn body_str(resp: axum::http::Response<Body>) -> String {
        let b = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(b.to_vec()).unwrap()
    }

    async fn seed_refresh_token(pool: &SqlitePool, token: &str, email: &str) {
        sqlx::query(
            "INSERT INTO refresh_tokens (token, email, expires_at) \
             VALUES (?, ?, datetime('now', '+365 days'))",
        )
        .bind(token)
        .bind(email)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn get_access_token(state: &AppState, refresh_token: &str) -> String {
        let resp = build_app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/token")
                    .header("authorization", format!("Bearer {refresh_token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK, "expected 200 from /token");
        let body = body_str(resp).await;
        let v: serde_json::Value = serde_json::from_str(&body).unwrap();
        v["access_token"].as_str().unwrap().to_string()
    }

    // --- /health ---

    #[tokio::test]
    async fn health_returns_ok() {
        let (state, _pool) = test_state().await;
        let resp = build_app(state)
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert!(body_str(resp).await.contains("\"ok\""));
    }

    // --- /metrics ---

    #[tokio::test]
    async fn metrics_content_type_and_format() {
        let (state, _pool) = test_state().await;
        let resp = build_app(state)
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let ct = resp.headers()["content-type"].to_str().unwrap();
        assert!(ct.contains("text/plain"));
        assert!(ct.contains("0.0.4"));
        let body = body_str(resp).await;
        assert!(body.contains("ccflux_reports_accepted_total"));
        assert!(body.contains("ccflux_active_access_tokens"));
    }

    // --- /token ---

    #[tokio::test]
    async fn token_unknown_refresh_returns_401() {
        let (state, _pool) = test_state().await;
        let resp = build_app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/token")
                    .header("authorization", "Bearer unknown-token")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn token_valid_refresh_returns_access_token() {
        let (state, pool) = test_state().await;
        seed_refresh_token(&pool, "rtok_test", "user@example.org").await;
        let access = get_access_token(&state, "rtok_test").await;
        assert!(!access.is_empty());
    }

    // --- /report ---

    #[tokio::test]
    async fn report_no_auth_returns_401() {
        let (state, _pool) = test_state().await;
        let resp = build_app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/report")
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn report_valid_unsigned_accepted_when_not_required() {
        let (state, pool) = test_state().await;
        seed_refresh_token(&pool, "rtok_report", "reporter@example.org").await;
        let access = get_access_token(&state, "rtok_report").await;

        let payload = serde_json::json!({
            "schema_version": 1,
            "session_id": "sess-abc",
            "user_email": "reporter@example.org",
            "turn_index": 0,
            "timestamp_utc": "2026-05-11T03:00:00Z",
            "models": {
                "claude-sonnet-4-6": {
                    "input_tokens": 100, "output_tokens": 50,
                    "cache_read_tokens": 0, "cache_write_tokens": 0
                }
            },
            "plugin_version": "0.1.0"
        });

        let resp = build_app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/report")
                    .header("authorization", format!("Bearer {access}"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn report_missing_sig_rejected_when_required() {
        let (state, pool) = test_state().await;
        // Override require_signatures
        let state = AppState {
            require_signatures: true,
            ..state
        };
        seed_refresh_token(&pool, "rtok_sig", "siguser@example.org").await;
        let access = get_access_token(&state, "rtok_sig").await;

        let payload = serde_json::json!({
            "schema_version": 1, "session_id": "sess-sig", "user_email": "siguser@example.org",
            "turn_index": 0, "timestamp_utc": "2026-05-11T03:00:00Z",
            "models": {"claude-sonnet-4-6": {"input_tokens":1,"output_tokens":1,"cache_read_tokens":0,"cache_write_tokens":0}},
            "plugin_version": "0.1.0"
        });

        let resp = build_app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/report")
                    .header("authorization", format!("Bearer {access}"))
                    .header("content-type", "application/json")
                    .body(Body::from(payload.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let err = resp
            .headers()
            .get("x-ccflux-error")
            .unwrap()
            .to_str()
            .unwrap();
        assert_eq!(err, "signature-required");
    }

    #[tokio::test]
    async fn report_full_signed_flow() {
        let (state, pool) = test_state().await;
        let state = AppState {
            require_signatures: true,
            ..state
        };
        seed_refresh_token(&pool, "rtok_full", "fulluser@example.org").await;
        let access = get_access_token(&state, "rtok_full").await;

        // Generate a deterministic test key
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let pubkey_b64 = STANDARD.encode(verifying_key.to_bytes());

        // Register the key
        let reg_resp = build_app(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/register-key")
                    .header("authorization", format!("Bearer {access}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        serde_json::json!({"public_key": pubkey_b64, "device_id": "test-host"})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(reg_resp.status(), StatusCode::OK);

        // Build and sign the report payload
        let payload = serde_json::json!({
            "schema_version": 1, "session_id": "sess-full", "user_email": "fulluser@example.org",
            "turn_index": 0, "timestamp_utc": "2026-05-11T03:00:00Z",
            "models": {"claude-sonnet-4-6": {"input_tokens":10,"output_tokens":5,"cache_read_tokens":0,"cache_write_tokens":0}},
            "plugin_version": "0.1.0"
        });
        let body_bytes = payload.to_string();
        let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);

        let mut msg = body_bytes.as_bytes().to_vec();
        msg.push(b'\n');
        msg.extend_from_slice(timestamp.as_bytes());
        let sig = signing_key.sign(&msg);
        let sig_header = format!("ed25519 {} {}", STANDARD.encode(sig.to_bytes()), pubkey_b64);

        let resp = build_app(state)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/report")
                    .header("authorization", format!("Bearer {access}"))
                    .header("content-type", "application/json")
                    .header("x-ccflux-signature", sig_header)
                    .header("x-ccflux-timestamp", &timestamp)
                    .body(Body::from(body_bytes))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
