use axum::{extract::State, http::HeaderMap, http::StatusCode, Json};

use crate::{db, model::RegisterKeyRequest, AppState};

pub async fn handle_register_key(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<RegisterKeyRequest>,
) -> StatusCode {
    let access_token = match extract_bearer(&headers) {
        Some(t) => t,
        None => return StatusCode::UNAUTHORIZED,
    };

    if !state.rate_limiter.allow(&access_token).await {
        return StatusCode::TOO_MANY_REQUESTS;
    }

    // Resolve email from the access token.
    let email = match db::email_from_access_token(&state.pool, &access_token).await {
        Ok(Some(e)) => e,
        Ok(None) => return StatusCode::UNAUTHORIZED,
        Err(e) => {
            eprintln!("register_key db error: {e}");
            return StatusCode::INTERNAL_SERVER_ERROR;
        }
    };

    match db::register_device_key(&state.pool, &email, &payload.public_key, &payload.device_id)
        .await
    {
        Ok(()) => {
            state.metrics.inc(&state.metrics.key_registrations);
            StatusCode::OK
        }
        Err(e) => {
            eprintln!("register_device_key error: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        }
    }
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let val = headers.get("authorization")?.to_str().ok()?;
    val.strip_prefix("Bearer ").map(|s| s.to_string())
}
