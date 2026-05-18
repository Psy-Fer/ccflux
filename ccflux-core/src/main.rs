use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

mod auth;
mod model;
mod offset;
mod parse;
mod queue;
mod report;
mod signing;

use model::{ClaudeConfig, HookInput, OffsetState, PluginConfig, UsagePayload};
use report::ReportStatus;

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

    // Generate the signing keypair on first session if not already present.
    // Registration happens in run_report once we have credentials.
    signing::load_or_generate(&data_dir);
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

    // If the device key has been revoked by IT, go silent with a logged message.
    if signing::is_revoked(&data_dir) {
        offset::log_error(
            &data_dir,
            "ccflux: device key revoked — contact your IT admin to re-provision",
        );
        return;
    }

    let (endpoint, access_token) = match resolve_credentials(&data_dir) {
        Some(pair) => pair,
        None => return,
    };

    // Load or generate the device signing key.
    let device_key = signing::load_or_generate(&data_dir);

    // If not yet registered, attempt registration on every turn until it succeeds.
    if !signing::is_registered(&data_dir) {
        signing::try_register(&data_dir, &endpoint, &access_token, &device_key);
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
        if ts.is_empty() {
            None
        } else {
            Some(ts)
        }
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

    let body = match serde_json::to_string(&payload) {
        Ok(b) => b,
        Err(e) => {
            offset::log_error(&data_dir, &format!("serialize: {e}"));
            return;
        }
    };

    let queue_path = offset::pending_reports_path(&data_dir);

    if signing::is_registered(&data_dir) {
        match report::post(&endpoint, &access_token, &body, &device_key) {
            ReportStatus::Accepted => {
                offset::log_activity(
                    &data_dir,
                    &format!(
                        "report: turn {} session {}… sent ok",
                        payload.turn_index,
                        &hook.session_id[..8]
                    ),
                );
                advance_offset(
                    &data_dir,
                    &hook.session_id,
                    &state,
                    turn_data.new_line,
                    is_session_end,
                );
                drain_one_queued(&data_dir, &endpoint, &access_token, &device_key);
            }
            ReportStatus::KeyRevoked => {
                offset::log_error(
                    &data_dir,
                    "ccflux: device key revoked — contact your IT admin to re-provision",
                );
                queue::clear(&queue_path);
                signing::mark_revoked(&data_dir);
            }
            ReportStatus::TimestampStale => {
                // Shouldn't happen for live reports (clock skew >5min is unusual).
                offset::log_error(
                    &data_dir,
                    "ccflux: request rejected as timestamp-stale (clock skew?)",
                );
            }
            ReportStatus::SignatureInvalid => {
                offset::log_error(
                    &data_dir,
                    "ccflux: signature-invalid — this is unexpected, retrying next turn",
                );
            }
            ReportStatus::KeyNotRegistered => {
                // Race condition: mark as unregistered and queue for next turn.
                let _ = std::fs::remove_file(
                    offset::pending_reports_path(&data_dir).with_file_name("key_registered"),
                );
                queue::enqueue(&queue_path, &body);
                advance_offset(
                    &data_dir,
                    &hook.session_id,
                    &state,
                    turn_data.new_line,
                    is_session_end,
                );
            }
            ReportStatus::Failed(e) => {
                offset::log_error(&data_dir, &format!("POST failed: {e}"));
            }
        }
    } else {
        // Key not yet registered: store locally and advance offset.
        queue::enqueue(&queue_path, &body);
        offset::log_activity(
            &data_dir,
            &format!(
                "report: turn {} session {}… queued (key not yet registered)",
                payload.turn_index,
                &hook.session_id[..8]
            ),
        );
        advance_offset(
            &data_dir,
            &hook.session_id,
            &state,
            turn_data.new_line,
            is_session_end,
        );
    }
}

fn drain_one_queued(data_dir: &Path, endpoint: &str, access_token: &str, key: &signing::DeviceKey) {
    let queue_path = offset::pending_reports_path(data_dir);
    if let Some(queued_body) = queue::drain_one(&queue_path) {
        match report::post(endpoint, access_token, &queued_body, key) {
            ReportStatus::Accepted => {
                offset::log_activity(data_dir, "report: drained 1 queued report ok");
            }
            ReportStatus::TimestampStale => {
                // The queued payload's event time may be old, but the HTTP timestamp
                // we send is always fresh. If the receiver still rejects it, discard —
                // there's no way to send it without a valid timestamp window.
                offset::log_error(
                    data_dir,
                    "ccflux: queued report rejected as timestamp-stale, discarding",
                );
            }
            ReportStatus::KeyRevoked => {
                offset::log_error(
                    data_dir,
                    "ccflux: device key revoked — contact your IT admin to re-provision",
                );
                queue::clear(&queue_path);
                signing::mark_revoked(data_dir);
            }
            ReportStatus::Failed(e) => {
                // Put it back at the front of the queue.
                let existing = std::fs::read_to_string(&queue_path).unwrap_or_default();
                let requeued = format!("{queued_body}\n{existing}");
                let _ = std::fs::write(&queue_path, requeued.as_bytes());
                offset::log_error(data_dir, &format!("queued POST failed: {e}"));
            }
            _ => {}
        }
    }
}

fn advance_offset(
    data_dir: &Path,
    session_id: &str,
    state: &OffsetState,
    new_line: usize,
    is_session_end: bool,
) {
    let new_state = OffsetState {
        line: new_line,
        turn: state.turn + 1,
        session_start: state.session_start.clone(),
        closed: is_session_end,
    };
    if let Err(e) = offset::write_offset(data_dir, session_id, &new_state) {
        offset::log_error(data_dir, &format!("offset write: {e}"));
    }
}

fn mark_closed(data_dir: &Path, session_id: &str, mut state: OffsetState) {
    state.closed = true;
    let _ = offset::write_offset(data_dir, session_id, &state);
}

fn resolve_credentials(data_dir: &Path) -> Option<(String, String)> {
    let mut endpoint = std::env::var("CLAUDE_PLUGIN_OPTION_API_ENDPOINT").unwrap_or_default();
    let mut refresh_token = std::env::var("CLAUDE_PLUGIN_OPTION_API_TOKEN").unwrap_or_default();

    if endpoint.is_empty() || refresh_token.is_empty() {
        if let Some(cfg) = read_plugin_config(data_dir) {
            if endpoint.is_empty() {
                endpoint = cfg.endpoint.unwrap_or_default();
            }
            if refresh_token.is_empty() {
                refresh_token = cfg.token.unwrap_or_default();
            }
        }
    }

    if endpoint.is_empty() || refresh_token.is_empty() {
        offset::log_activity(
            data_dir,
            &format!(
                "no credentials — create {} with endpoint and token to enable reporting",
                offset::config_path(data_dir).display()
            ),
        );
        return None;
    }

    let access_token = auth::get_access_token(data_dir, &endpoint, &refresh_token)?;
    Some((endpoint, access_token))
}

fn data_dir_from_transcript(transcript: &Path) -> Option<PathBuf> {
    transcript
        .parent()?
        .parent()?
        .parent()
        .map(|p| p.to_path_buf())
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
