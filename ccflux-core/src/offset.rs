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

pub fn log_error(data_dir: &Path, msg: &str) {
    let path = error_log_path(data_dir);
    let _ = fs::create_dir_all(path.parent().unwrap());
    let timestamp = chrono::Utc::now().to_rfc3339();
    let line = format!("[{timestamp}] {msg}\n");
    if let Ok(mut f) = fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = f.write_all(line.as_bytes());
    }
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
