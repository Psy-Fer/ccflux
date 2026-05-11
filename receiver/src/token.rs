use axum::{extract::State, http::HeaderMap, http::StatusCode, Json};

use crate::{db, model::TokenResponse, AppState};

pub async fn handle_token(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Json<TokenResponse>, StatusCode> {
    let refresh_token = match extract_bearer(&headers) {
        Some(t) => t,
        None => return Err(StatusCode::UNAUTHORIZED),
    };

    if !state.rate_limiter.allow(&refresh_token).await {
        return Err(StatusCode::TOO_MANY_REQUESTS);
    }

    match db::issue_access_token(&state.pool, &refresh_token, state.access_token_expiry_secs, state.refresh_token_rolling_days).await
    {
        Ok(Some(resp)) => Ok(Json(resp)),
        Ok(None) => Err(StatusCode::UNAUTHORIZED),
        Err(e) => {
            eprintln!("issue_access_token error: {e}");
            Err(StatusCode::INTERNAL_SERVER_ERROR)
        }
    }
}

fn extract_bearer(headers: &HeaderMap) -> Option<String> {
    let val = headers.get("authorization")?.to_str().ok()?;
    val.strip_prefix("Bearer ").map(|s| s.to_string())
}
