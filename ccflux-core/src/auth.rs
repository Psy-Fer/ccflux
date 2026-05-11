use std::path::Path;
use std::time::Duration;

use crate::model::{TokenCache, TokenResponse};
use crate::offset;

/// How far before expiry we proactively refresh (5 minutes).
const REFRESH_BUFFER_SECS: i64 = 300;

/// Returns a valid short-lived access token, using the cache when possible.
/// On any failure, logs to errors.log and returns None so the hook exits silently.
pub fn get_access_token(
    data_dir: &Path,
    report_endpoint: &str,
    refresh_token: &str,
) -> Option<String> {
    if let Some(cached) = read_cache(data_dir) {
        if !is_expiring_soon(&cached.expires_at) {
            return Some(cached.access_token);
        }
    }

    let token_url = token_endpoint(report_endpoint);
    match exchange(data_dir, &token_url, refresh_token) {
        Ok(access_token) => Some(access_token),
        Err(e) => {
            offset::log_error(data_dir, &format!("token refresh failed: {e}"));
            None
        }
    }
}

/// Derives the /token endpoint from the /report endpoint URL by replacing
/// the last path component. https://host/report → https://host/token,
/// https://host/api/report → https://host/api/token.
fn token_endpoint(report_endpoint: &str) -> String {
    // More than 2 slashes means there is a path component (not just https://host).
    if report_endpoint.matches('/').count() > 2 {
        let pos = report_endpoint.rfind('/').unwrap();
        format!("{}/token", &report_endpoint[..pos])
    } else {
        format!("{}/token", report_endpoint)
    }
}

fn is_expiring_soon(expires_at: &str) -> bool {
    let Ok(expiry) = chrono::DateTime::parse_from_rfc3339(expires_at) else {
        return true;
    };
    let threshold = chrono::Utc::now() + chrono::Duration::seconds(REFRESH_BUFFER_SECS);
    expiry.with_timezone(&chrono::Utc) <= threshold
}

fn exchange(data_dir: &Path, token_url: &str, refresh_token: &str) -> Result<String, String> {
    if !token_url.starts_with("https://") {
        return Err(format!(
            "token endpoint must use https://, got: {token_url}"
        ));
    }

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(5))
        .build();

    let resp = agent
        .post(token_url)
        .set("Authorization", &format!("Bearer {refresh_token}"))
        .call()
        .map_err(|e| format!("POST {token_url}: {e}"))?;

    let status = resp.status();
    if status == 401 {
        return Err(
            "refresh token expired or revoked — contact your IT admin to issue a new one"
                .to_string(),
        );
    }
    if status >= 300 {
        return Err(format!("token endpoint returned HTTP {status}"));
    }

    let body = resp
        .into_string()
        .map_err(|e| format!("read /token response: {e}"))?;
    let token_resp: TokenResponse =
        serde_json::from_str(&body).map_err(|e| format!("parse /token response: {e}"))?;

    let cache = TokenCache {
        access_token: token_resp.access_token.clone(),
        expires_at: token_resp.expires_at,
    };
    write_cache(data_dir, &cache);

    Ok(token_resp.access_token)
}

fn read_cache(data_dir: &Path) -> Option<TokenCache> {
    let content = std::fs::read_to_string(offset::token_cache_path(data_dir)).ok()?;
    serde_json::from_str(&content).ok()
}

fn write_cache(data_dir: &Path, cache: &TokenCache) {
    let path = offset::token_cache_path(data_dir);
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, serde_json::to_string(cache).unwrap());
    offset::set_secure_permissions(&path);
}
