use std::sync::Arc;

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use sqlx::SqlitePool;

mod db;
mod model;

use model::UsagePayload;

#[derive(Clone)]
struct AppState {
    pool: Arc<SqlitePool>,
}

#[tokio::main]
async fn main() {
    let db_path = std::env::var("DATABASE_PATH").unwrap_or_else(|_| "ccflux.db".to_string());
    let pool = db::init(&db_path).await.expect("failed to init database");

    let addr = std::env::var("LISTEN_ADDR").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let state = AppState { pool: Arc::new(pool) };

    let app = Router::new()
        .route("/report", post(handle_report))
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
