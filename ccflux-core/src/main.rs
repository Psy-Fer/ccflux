use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

mod model;
mod offset;
mod parse;
mod report;

use model::{ClaudeConfig, HookInput, OffsetState, PluginConfig, UsagePayload};

#[derive(Parser)]
#[command(name = "ccflux")]
struct Cli {
    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    ReportTurn {
        #[arg(long)]
        input: String,
    },
    SessionEnd {
        #[arg(long)]
        input: String,
    },
    Init {
        #[arg(long)]
        input: String,
    },
}

fn main() {
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(_) => return,
    };

    match cli.command {
        Cmd::ReportTurn { input } => run_report(&input, false),
        Cmd::SessionEnd { input } => run_report(&input, true),
        Cmd::Init { input } => run_init(&input),
    }
}

fn run_init(input: &str) {
    let hook: HookInput = match serde_json::from_str(input) {
        Ok(h) => h,
        Err(_) => return,
    };
    let transcript = Path::new(&hook.transcript_path);
    let data_dir = match data_dir_from_transcript(transcript) {
        Some(d) => d,
        None => return,
    };
    let session_start = parse::first_timestamp(transcript);
    let _ = offset::init_offset(&data_dir, &hook.session_id, &session_start);
}

fn run_report(input: &str, is_session_end: bool) {
    let hook: HookInput = match serde_json::from_str(input) {
        Ok(h) => h,
        Err(_) => return,
    };

    let transcript = Path::new(&hook.transcript_path);
    let data_dir = match data_dir_from_transcript(transcript) {
        Some(d) => d,
        None => return,
    };

    if !transcript_belongs_to_plugin(&data_dir) {
        return;
    }

    let (endpoint, token) = match resolve_credentials(&data_dir) {
        Some(pair) => pair,
        None => return,
    };

    let state = offset::read_offset(&data_dir, &hook.session_id);

    let turn_data = match parse::collect_since_offset(transcript, &state) {
        Ok(Some(d)) => d,
        Ok(None) => {
            if is_session_end {
                mark_closed(&data_dir, &hook.session_id, state);
            }
            return;
        }
        Err(e) => {
            offset::log_error(&data_dir, &format!("parse: {e}"));
            return;
        }
    };

    let email = read_email(&data_dir);
    let session_start = if state.session_start.is_empty() {
        let ts = parse::first_timestamp(transcript);
        if ts.is_empty() { None } else { Some(ts) }
    } else {
        Some(state.session_start.clone())
    };

    let payload = UsagePayload {
        schema_version: 1,
        session_id: hook.session_id.clone(),
        user_email: email,
        turn_index: state.turn,
        timestamp_utc: turn_data.timestamp,
        session_start_utc: session_start,
        models: turn_data.models,
        plugin_version: env!("CARGO_PKG_VERSION").to_string(),
    };

    match report::post(&endpoint, &token, &payload) {
        Ok(()) => {
            let new_state = OffsetState {
                line: turn_data.new_line,
                turn: state.turn + 1,
                session_start: state.session_start,
                closed: is_session_end,
            };
            if let Err(e) = offset::write_offset(&data_dir, &hook.session_id, &new_state) {
                offset::log_error(&data_dir, &format!("offset write: {e}"));
            }
        }
        Err(e) => {
            offset::log_error(&data_dir, &format!("POST failed: {e}"));
        }
    }
}

fn mark_closed(data_dir: &Path, session_id: &str, mut state: OffsetState) {
    state.closed = true;
    let _ = offset::write_offset(data_dir, session_id, &state);
}

/// Resolves endpoint + token without exposing them as CLI args.
/// Priority: CLAUDE_PLUGIN_OPTION_* env vars → config.json.
/// Returns None if either value is absent after both sources.
fn resolve_credentials(data_dir: &Path) -> Option<(String, String)> {
    let mut endpoint = std::env::var("CLAUDE_PLUGIN_OPTION_API_ENDPOINT").unwrap_or_default();
    let mut token = std::env::var("CLAUDE_PLUGIN_OPTION_API_TOKEN").unwrap_or_default();

    if endpoint.is_empty() || token.is_empty() {
        if let Some(cfg) = read_plugin_config(data_dir) {
            if endpoint.is_empty() {
                endpoint = cfg.endpoint.unwrap_or_default();
            }
            if token.is_empty() {
                token = cfg.token.unwrap_or_default();
            }
        }
    }

    if endpoint.is_empty() || token.is_empty() {
        return None;
    }
    Some((endpoint, token))
}

fn data_dir_from_transcript(transcript: &Path) -> Option<PathBuf> {
    transcript.parent()?.parent()?.parent().map(|p| p.to_path_buf())
}

fn transcript_belongs_to_plugin(transcript_data_dir: &Path) -> bool {
    let plugin_root = match std::env::var("CLAUDE_PLUGIN_ROOT") {
        Ok(v) => v,
        Err(_) => return true,
    };
    let components: Vec<_> = Path::new(&plugin_root).components().collect();
    for (i, component) in components.iter().enumerate() {
        if component.as_os_str() == "plugins" && i > 0 {
            let plugin_data_dir: PathBuf = components[..i].iter().collect();
            return transcript_data_dir == plugin_data_dir;
        }
    }
    true
}

fn read_email(data_dir: &Path) -> String {
    let content = std::fs::read_to_string(data_dir.join(".claude.json")).unwrap_or_default();
    let cfg: ClaudeConfig = serde_json::from_str(&content).unwrap_or_default();
    cfg.oauth_account
        .and_then(|a| a.email_address)
        .unwrap_or_default()
}

fn read_plugin_config(data_dir: &Path) -> Option<PluginConfig> {
    let content = std::fs::read_to_string(offset::config_path(data_dir)).ok()?;
    serde_json::from_str(&content).ok()
}
