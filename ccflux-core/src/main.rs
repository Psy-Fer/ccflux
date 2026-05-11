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
    /// Called by the Stop hook after each assistant turn.
    ReportTurn {
        #[arg(long)]
        input: String,
        #[arg(long, default_value = "")]
        endpoint: String,
        #[arg(long, default_value = "")]
        token: String,
    },
    /// Called by the SessionEnd hook; same as report-turn but marks session closed.
    SessionEnd {
        #[arg(long)]
        input: String,
        #[arg(long, default_value = "")]
        endpoint: String,
        #[arg(long, default_value = "")]
        token: String,
    },
    /// Called by the SessionStart hook; creates the offset sidecar file.
    Init {
        #[arg(long)]
        input: String,
    },
}

fn main() {
    // try_parse returns Err for --help, bad args, etc. — always exit 0.
    let cli = match Cli::try_parse() {
        Ok(c) => c,
        Err(_) => return,
    };

    match cli.command {
        Cmd::ReportTurn { input, endpoint, token } => run_report(&input, endpoint, token, false),
        Cmd::SessionEnd { input, endpoint, token } => run_report(&input, endpoint, token, true),
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

fn run_report(input: &str, mut endpoint: String, mut token: String, is_session_end: bool) {
    let hook: HookInput = match serde_json::from_str(input) {
        Ok(h) => h,
        Err(_) => return,
    };

    let transcript = Path::new(&hook.transcript_path);
    let data_dir = match data_dir_from_transcript(transcript) {
        Some(d) => d,
        None => return,
    };

    // Safety check: the transcript must belong to the same CC config dir as this plugin.
    if !transcript_belongs_to_plugin(&data_dir) {
        return;
    }

    // Resolve endpoint and token: CLI args (from CLAUDE_PLUGIN_OPTION_* env vars) take
    // priority; fall back to <data_dir>/ccflux/config.json for pre-configured deployments.
    if endpoint.is_empty() || token.is_empty() {
        if let Some(cfg) = read_plugin_config(&data_dir) {
            if endpoint.is_empty() {
                endpoint = cfg.endpoint.unwrap_or_default();
            }
            if token.is_empty() {
                token = cfg.token.unwrap_or_default();
            }
        }
    }
    if endpoint.is_empty() || token.is_empty() {
        return;
    }

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

/// Derives the CC data dir from the transcript path.
/// transcript = <data_dir>/projects/<project_hash>/<session_id>.jsonl
fn data_dir_from_transcript(transcript: &Path) -> Option<PathBuf> {
    transcript.parent()?.parent()?.parent().map(|p| p.to_path_buf())
}

/// Verifies that the transcript's data dir matches the CC instance that installed this plugin.
/// Prevents a plugin installed in ~/.claude-work from reporting on ~/.claude sessions.
/// If CLAUDE_PLUGIN_ROOT is not set (e.g. during testing), the check is skipped.
fn transcript_belongs_to_plugin(transcript_data_dir: &Path) -> bool {
    let plugin_root = match std::env::var("CLAUDE_PLUGIN_ROOT") {
        Ok(v) => v,
        Err(_) => return true,
    };

    // Walk up the plugin root path to find the first 'plugins' component.
    // Everything before it is the CC data dir.
    let components: Vec<_> = Path::new(&plugin_root).components().collect();
    for (i, component) in components.iter().enumerate() {
        if component.as_os_str() == "plugins" && i > 0 {
            let plugin_data_dir: PathBuf = components[..i].iter().collect();
            return transcript_data_dir == plugin_data_dir;
        }
    }

    // Can't determine — don't block
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
