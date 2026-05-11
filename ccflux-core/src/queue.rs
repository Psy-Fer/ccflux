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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn qpath(dir: &TempDir) -> std::path::PathBuf {
        dir.path().join("pending.jsonl")
    }

    #[test]
    fn roundtrip() {
        let dir = TempDir::new().unwrap();
        let p = qpath(&dir);
        enqueue(&p, "hello");
        assert_eq!(drain_one(&p), Some("hello".to_string()));
        assert_eq!(drain_one(&p), None);
    }

    #[test]
    fn drain_empty_returns_none() {
        let dir = TempDir::new().unwrap();
        assert_eq!(drain_one(&qpath(&dir)), None);
    }

    #[test]
    fn fifo_order() {
        let dir = TempDir::new().unwrap();
        let p = qpath(&dir);
        enqueue(&p, "a");
        enqueue(&p, "b");
        enqueue(&p, "c");
        assert_eq!(drain_one(&p).unwrap(), "a");
        assert_eq!(drain_one(&p).unwrap(), "b");
        assert_eq!(drain_one(&p).unwrap(), "c");
        assert_eq!(drain_one(&p), None);
    }

    #[test]
    fn cap_drops_oldest() {
        let dir = TempDir::new().unwrap();
        let p = qpath(&dir);
        for i in 0..MAX_ENTRIES {
            enqueue(&p, &format!("entry_{i}"));
        }
        // One more should evict entry_0
        enqueue(&p, "overflow");
        assert_eq!(drain_one(&p).unwrap(), "entry_1");
    }

    #[test]
    fn clear_empties_queue() {
        let dir = TempDir::new().unwrap();
        let p = qpath(&dir);
        enqueue(&p, "a");
        enqueue(&p, "b");
        clear(&p);
        assert_eq!(drain_one(&p), None);
    }
}
