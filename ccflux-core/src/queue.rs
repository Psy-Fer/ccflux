use std::path::Path;

const MAX_ENTRIES: usize = 500;

/// Appends a JSON payload string to the pending queue.
/// Enforces the entry cap by dropping the oldest entry when full.
pub fn enqueue(path: &Path, payload_json: &str) {
    let _ = std::fs::create_dir_all(path.parent().unwrap());

    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let mut lines: Vec<&str> = existing.lines().filter(|l| !l.is_empty()).collect();

    if lines.len() >= MAX_ENTRIES {
        lines.remove(0);
    }

    let mut content: String = lines.join("\n");
    if !content.is_empty() {
        content.push('\n');
    }
    content.push_str(payload_json);
    content.push('\n');

    let _ = std::fs::write(path, content.as_bytes());
}

/// Removes and returns the oldest entry from the queue, or None if empty.
pub fn drain_one(path: &Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return None;
    }
    let first = lines.remove(0).to_string();
    let remaining = if lines.is_empty() {
        String::new()
    } else {
        format!("{}\n", lines.join("\n"))
    };
    let _ = std::fs::write(path, remaining.as_bytes());
    Some(first)
}

/// Removes all queued entries.
pub fn clear(path: &Path) {
    let _ = std::fs::remove_file(path);
}
