use std::fs;
use std::io::Write as IoWrite;
use std::path::{Path, PathBuf};

use crate::model::OffsetState;

pub fn offset_path(data_dir: &Path, session_id: &str) -> PathBuf {
    data_dir.join("ccflux").join(format!("{session_id}.offset"))
}

pub fn error_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ccflux").join("errors.log")
}

pub fn config_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ccflux").join("config.json")
}

pub fn token_cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ccflux").join("token_cache.json")
}

pub fn pending_reports_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ccflux").join("pending_reports.jsonl")
}

pub fn read_offset(data_dir: &Path, session_id: &str) -> OffsetState {
    let path = offset_path(data_dir, session_id);
    let content = fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&content).unwrap_or_default()
}

pub fn write_offset(data_dir: &Path, session_id: &str, state: &OffsetState) -> std::io::Result<()> {
    let path = offset_path(data_dir, session_id);
    let tmp = path.with_extension("tmp");
    fs::create_dir_all(path.parent().unwrap())?;
    fs::write(&tmp, serde_json::to_string(state).unwrap())?;
    set_secure_permissions(&tmp);
    fs::rename(tmp, path)
}

pub fn init_offset(data_dir: &Path, session_id: &str, session_start: &str) -> std::io::Result<()> {
    let path = offset_path(data_dir, session_id);
    if path.exists() {
        return Ok(());
    }
    let state = OffsetState {
        session_start: session_start.to_string(),
        ..Default::default()
    };
    write_offset(data_dir, session_id, &state)
}

pub fn activity_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ccflux").join("activity.log")
}

fn append_log(path: &Path, line: &str) {
    let _ = fs::create_dir_all(path.parent().unwrap());
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = f.write_all(line.as_bytes());
    }
    // Cap at ~64 KB: keep the last half when exceeded.
    if let Ok(meta) = fs::metadata(path) {
        if meta.len() > 65536 {
            if let Ok(content) = fs::read_to_string(path) {
                let lines: Vec<&str> = content.lines().collect();
                let keep = lines.len() / 2;
                let trimmed = lines[lines.len() - keep..].join("\n") + "\n";
                let _ = fs::write(path, trimmed);
            }
        }
    }
}

pub fn log_error(data_dir: &Path, msg: &str) {
    let timestamp = chrono::Local::now().to_rfc3339();
    let line = format!("[{timestamp}] {msg}\n");
    append_log(&error_log_path(data_dir), &line);
    append_log(
        &activity_log_path(data_dir),
        &format!("[{timestamp}] ERROR {msg}\n"),
    );
}

pub fn log_activity(data_dir: &Path, msg: &str) {
    let timestamp = chrono::Local::now().to_rfc3339();
    let line = format!("[{timestamp}] {msg}\n");
    append_log(&activity_log_path(data_dir), &line);
}

/// Sets file permissions to owner-read/write only (0600 on Unix). No-op elsewhere.
pub fn set_secure_permissions(path: &Path) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    #[cfg(not(unix))]
    let _ = path;
}
