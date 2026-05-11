use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Mirrors UsagePayload in ccflux-core. Keep in sync.
#[derive(Deserialize)]
pub struct UsagePayload {
    pub schema_version: u32,
    pub session_id: String,
    #[allow(dead_code)]  // present in payload but identity is resolved from the access token
    pub user_email: String,
    pub turn_index: u64,
    pub timestamp_utc: String,
    pub session_start_utc: Option<String>,
    pub models: HashMap<String, ModelUsage>,
    pub plugin_version: Option<String>,
}

#[derive(Deserialize)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

/// Returned by POST /token.
#[derive(Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub expires_at: String,
    pub token_type: String,
}

/// Body of POST /register-key.
#[derive(Deserialize)]
pub struct RegisterKeyRequest {
    pub public_key: String,
    pub device_id: String,
}
