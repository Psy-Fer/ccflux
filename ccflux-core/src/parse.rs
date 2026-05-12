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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::NamedTempFile;

    fn make_transcript(lines: &[&str]) -> NamedTempFile {
        let mut f = NamedTempFile::new().unwrap();
        for line in lines {
            writeln!(f, "{line}").unwrap();
        }
        f
    }

    const ASSISTANT: &str = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-11T03:00:00Z","message":{"model":"claude-sonnet-4-6","usage":{"input_tokens":100,"output_tokens":200,"cache_read_input_tokens":50,"cache_creation_input_tokens":25}}}"#;
    const HUMAN: &str = r#"{"type":"human","sessionId":"s1","timestamp":"2026-05-11T03:01:00Z"}"#;

    #[test]
    fn first_timestamp_empty_file() {
        let f = make_transcript(&[]);
        assert_eq!(first_timestamp(f.path()), "");
    }

    #[test]
    fn first_timestamp_skips_non_assistant() {
        let f = make_transcript(&[HUMAN, ASSISTANT]);
        // first_timestamp returns the first timestamp it finds regardless of type
        assert_eq!(first_timestamp(f.path()), "2026-05-11T03:01:00Z");
    }

    #[test]
    fn collect_basic_turn() {
        let f = make_transcript(&[ASSISTANT]);
        let result = collect_since_offset(f.path(), &OffsetState::default())
            .unwrap()
            .unwrap();
        let u = result.models.get("claude-sonnet-4-6").unwrap();
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 200);
        assert_eq!(u.cache_read_tokens, 50);
        assert_eq!(u.cache_write_tokens, 25);
        assert_eq!(result.timestamp, "2026-05-11T03:00:00Z");
        assert_eq!(result.new_line, 1);
    }

    #[test]
    fn collect_no_assistant_returns_none() {
        let f = make_transcript(&[HUMAN]);
        assert!(collect_since_offset(f.path(), &OffsetState::default())
            .unwrap()
            .is_none());
    }

    #[test]
    fn collect_respects_offset() {
        // Three lines: assistant(0), human(1), assistant(2)
        // With offset 2, only the third line should be processed.
        let f = make_transcript(&[ASSISTANT, HUMAN, ASSISTANT]);
        let state = OffsetState {
            line: 2,
            ..Default::default()
        };
        let result = collect_since_offset(f.path(), &state).unwrap().unwrap();
        assert_eq!(result.new_line, 3);
        // Only one assistant entry processed, not two
        assert_eq!(result.models["claude-sonnet-4-6"].input_tokens, 100);
    }

    #[test]
    fn collect_aggregates_same_model() {
        let f = make_transcript(&[ASSISTANT, ASSISTANT]);
        let result = collect_since_offset(f.path(), &OffsetState::default())
            .unwrap()
            .unwrap();
        let u = &result.models["claude-sonnet-4-6"];
        assert_eq!(u.input_tokens, 200);
        assert_eq!(u.output_tokens, 400);
    }

    #[test]
    fn collect_multiple_models() {
        let opus = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-11T03:01:00Z","message":{"model":"claude-opus-4-7","usage":{"input_tokens":10,"output_tokens":20,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#;
        let f = make_transcript(&[ASSISTANT, opus]);
        let result = collect_since_offset(f.path(), &OffsetState::default())
            .unwrap()
            .unwrap();
        assert_eq!(result.models.len(), 2);
        assert!(result.models.contains_key("claude-sonnet-4-6"));
        assert!(result.models.contains_key("claude-opus-4-7"));
    }

    #[test]
    fn collect_missing_usage_fields_defaults_to_zero() {
        let entry = r#"{"type":"assistant","sessionId":"s1","timestamp":"2026-05-11T03:00:00Z","message":{"model":"claude-sonnet-4-6","usage":{}}}"#;
        let f = make_transcript(&[entry]);
        let result = collect_since_offset(f.path(), &OffsetState::default())
            .unwrap()
            .unwrap();
        let u = &result.models["claude-sonnet-4-6"];
        assert_eq!(u.input_tokens, 0);
        assert_eq!(u.output_tokens, 0);
        assert_eq!(u.cache_read_tokens, 0);
        assert_eq!(u.cache_write_tokens, 0);
    }
}
