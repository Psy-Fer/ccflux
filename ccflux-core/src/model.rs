use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// JSON passed on stdin by CC to every hook.
#[derive(Deserialize)]
pub struct HookInput {
    pub session_id: String,
    pub transcript_path: String,
}

/// A single line in the JSONL transcript.
#[derive(Deserialize)]
pub struct TranscriptEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub timestamp: Option<String>,
    pub message: Option<TranscriptMessage>,
}

#[derive(Deserialize)]
pub struct TranscriptMessage {
    pub model: Option<String>,
    pub usage: Option<UsageData>,
}

#[derive(Deserialize)]
pub struct UsageData {
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cache_read_input_tokens: Option<u64>,
    pub cache_creation_input_tokens: Option<u64>,
}

/// Payload POSTed to the receiver.
#[derive(Serialize)]
pub struct UsagePayload {
    pub schema_version: u32,
    pub session_id: String,
    pub user_email: String,
    pub turn_index: u64,
    pub timestamp_utc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_start_utc: Option<String>,
    pub models: HashMap<String, ModelUsage>,
    pub plugin_version: String,
}

#[derive(Serialize, Default)]
pub struct ModelUsage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
}

/// Persisted between hook invocations to track progress through the transcript.
#[derive(Serialize, Deserialize, Default)]
pub struct OffsetState {
    /// Last line number processed (0-based, exclusive end).
    pub line: usize,
    /// Number of turns successfully reported so far.
    pub turn: u64,
    /// ISO 8601 timestamp of the first JSONL entry (session start).
    pub session_start: String,
    /// Set to true by session-end.
    #[serde(default)]
    pub closed: bool,
}

/// Minimal parse of .claude.json to get the logged-in email.
#[derive(Deserialize, Default)]
pub struct ClaudeConfig {
    #[serde(rename = "oauthAccount")]
    pub oauth_account: Option<OauthAccount>,
}

#[derive(Deserialize)]
pub struct OauthAccount {
    #[serde(rename = "emailAddress")]
    pub email_address: Option<String>,
}

/// Optional pre-configured endpoint/token stored at <data_dir>/ccflux/config.json.
/// Used when CLAUDE_PLUGIN_OPTION_* env vars are not set.
#[derive(Deserialize)]
pub struct PluginConfig {
    pub endpoint: Option<String>,
    pub token: Option<String>,
}
