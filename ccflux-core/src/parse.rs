use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use crate::model::{ModelUsage, OffsetState, TranscriptEntry};

pub struct TurnUsage {
    pub models: HashMap<String, ModelUsage>,
    /// Timestamp of the last assistant entry found.
    pub timestamp: String,
    /// Line count after consuming all entries (next offset).
    pub new_line: usize,
}

/// Reads all assistant entries from `transcript` starting at `state.line`,
/// aggregates token usage by model, and returns the aggregated turn data.
/// Returns `None` if there are no new assistant entries.
pub fn collect_since_offset(
    transcript: &Path,
    state: &OffsetState,
) -> Result<Option<TurnUsage>, String> {
    let file =
        File::open(transcript).map_err(|e| format!("cannot open {}: {e}", transcript.display()))?;
    let reader = BufReader::new(file);

    let mut models: HashMap<String, ModelUsage> = HashMap::new();
    let mut last_timestamp = String::new();
    let mut line_num = 0usize;
    let mut found_any = false;

    for raw in reader.lines() {
        let raw = raw.map_err(|e| format!("read error at line {line_num}: {e}"))?;
        let current = line_num;
        line_num += 1;

        if current < state.line {
            continue;
        }

        let entry: TranscriptEntry = match serde_json::from_str(&raw) {
            Ok(e) => e,
            Err(_) => continue,
        };

        if entry.entry_type != "assistant" {
            continue;
        }

        let msg = match entry.message {
            Some(m) => m,
            None => continue,
        };

        let usage = match msg.usage {
            Some(u) => u,
            None => continue,
        };

        let model = msg.model.unwrap_or_else(|| "unknown".to_string());
        if let Some(ts) = entry.timestamp {
            last_timestamp = ts;
        }

        let mu = models.entry(model).or_default();
        mu.input_tokens += usage.input_tokens.unwrap_or(0);
        mu.output_tokens += usage.output_tokens.unwrap_or(0);
        mu.cache_read_tokens += usage.cache_read_input_tokens.unwrap_or(0);
        mu.cache_write_tokens += usage.cache_creation_input_tokens.unwrap_or(0);
        found_any = true;
    }

    if !found_any {
        return Ok(None);
    }

    Ok(Some(TurnUsage {
        models,
        timestamp: last_timestamp,
        new_line: line_num,
    }))
}

/// Reads up to the first 50 lines to find the earliest timestamp.
/// Used as fallback session_start when init didn't run.
pub fn first_timestamp(transcript: &Path) -> String {
    let file = match File::open(transcript) {
        Ok(f) => f,
        Err(_) => return String::new(),
    };
    for raw in BufReader::new(file).lines().take(50).flatten() {
        if let Ok(entry) = serde_json::from_str::<TranscriptEntry>(&raw) {
            if let Some(ts) = entry.timestamp {
                return ts;
            }
        }
    }
    String::new()
}
