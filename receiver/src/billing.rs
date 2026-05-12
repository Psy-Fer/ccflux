use std::collections::HashMap;

use chrono::{DateTime, TimeZone, Utc};

const WINDOW_SECS: i64 = 5 * 3600;

/// A single usage event fetched from the DB.
pub struct RawEvent {
    pub user_email: String,
    pub timestamp_utc: String,
    pub session_id: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
}

/// A computed 5-hour billing window for one user, matching the ccusage algorithm:
/// a new window begins when either the time since the window start OR the time
/// since the previous entry reaches 5 hours (inactivity reset).
pub struct BillingWindow {
    pub user_email: String,
    /// Window start — floored to the nearest UTC hour of the first entry.
    pub start: DateTime<Utc>,
    /// Nominal window end (start + 5 h).  The real CC limit may reset earlier
    /// on inactivity, but this is the outer bound.
    pub end: DateTime<Utc>,
    /// True when the last entry was < 5 h ago AND now is before `end`.
    pub is_active: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub turns: i64,
    pub session_count: i64,
    pub last_entry: DateTime<Utc>,
}

impl BillingWindow {
    pub fn billed_tokens(&self) -> i64 {
        self.input_tokens + self.output_tokens
    }
}

/// Groups events into billing windows using the ccusage algorithm.
/// Returns windows sorted by `last_entry` descending (most recent first).
pub fn compute_billing_windows(mut events: Vec<RawEvent>) -> Vec<BillingWindow> {
    let now = Utc::now();

    // Sort by user then timestamp so we can process each user in a single pass.
    events.sort_by(|a, b| {
        a.user_email
            .cmp(&b.user_email)
            .then(a.timestamp_utc.cmp(&b.timestamp_utc))
    });

    let mut result: Vec<BillingWindow> = Vec::new();

    let mut idx = 0;
    while idx < events.len() {
        let user = events[idx].user_email.clone();

        // Collect contiguous slice for this user.
        let start_idx = idx;
        while idx < events.len() && events[idx].user_email == user {
            idx += 1;
        }
        let user_events = &events[start_idx..idx];

        // Build windows for this user.
        let mut windows: Vec<BillingWindow> = Vec::new();
        // Track unique session IDs per window separately (to compute session_count).
        let mut session_sets: Vec<HashMap<String, ()>> = Vec::new();

        for ev in user_events {
            let Some(ts) = parse_ts(&ev.timestamp_utc) else {
                continue;
            };

            let needs_new = match windows.last() {
                None => true,
                Some(w) => {
                    (ts - w.start).num_seconds() >= WINDOW_SECS
                        || (ts - w.last_entry).num_seconds() >= WINDOW_SECS
                }
            };

            if needs_new {
                windows.push(BillingWindow {
                    user_email: user.clone(),
                    start: floor_to_hour(ts),
                    end: floor_to_hour(ts) + chrono::Duration::seconds(WINDOW_SECS),
                    is_active: false,
                    input_tokens: 0,
                    output_tokens: 0,
                    cache_read_tokens: 0,
                    cache_write_tokens: 0,
                    turns: 0,
                    session_count: 0,
                    last_entry: ts,
                });
                session_sets.push(HashMap::new());
            }

            let w = windows.last_mut().unwrap();
            w.input_tokens += ev.input_tokens;
            w.output_tokens += ev.output_tokens;
            w.cache_read_tokens += ev.cache_read_tokens;
            w.cache_write_tokens += ev.cache_write_tokens;
            w.turns += 1;
            w.last_entry = ts;
            session_sets.last_mut().unwrap().insert(ev.session_id.clone(), ());
        }

        for (w, sessions) in windows.iter_mut().zip(session_sets.iter()) {
            w.session_count = sessions.len() as i64;
            let since_last = (now - w.last_entry).num_seconds();
            let since_start = (now - w.start).num_seconds();
            w.is_active = since_last < WINDOW_SECS && since_start < WINDOW_SECS;
        }

        result.extend(windows);
    }

    result.sort_by_key(|w| std::cmp::Reverse(w.last_entry));
    result
}

fn floor_to_hour(dt: DateTime<Utc>) -> DateTime<Utc> {
    let secs = dt.timestamp();
    Utc.timestamp_opt(secs - secs % 3600, 0).unwrap()
}

fn parse_ts(s: &str) -> Option<DateTime<Utc>> {
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    // SQLite datetime('now') format: "YYYY-MM-DD HH:MM:SS"
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S") {
        return Some(DateTime::from_naive_utc_and_offset(dt, Utc));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(user: &str, ts: &str, session: &str, inp: i64, out: i64) -> RawEvent {
        RawEvent {
            user_email: user.to_string(),
            timestamp_utc: ts.to_string(),
            session_id: session.to_string(),
            input_tokens: inp,
            output_tokens: out,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
        }
    }

    #[test]
    fn single_session_within_5h_is_one_window() {
        let events = vec![
            ev("a@x", "2026-05-12T10:00:00Z", "s1", 100, 50),
            ev("a@x", "2026-05-12T11:00:00Z", "s1", 200, 80),
            ev("a@x", "2026-05-12T14:00:00Z", "s1", 150, 60), // still < 5h from start
        ];
        let windows = compute_billing_windows(events);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].turns, 3);
        assert_eq!(windows[0].input_tokens, 450);
        assert_eq!(windows[0].start, parse_ts("2026-05-12T10:00:00Z").unwrap()); // floored to 10:00
    }

    #[test]
    fn entries_spanning_5h_create_two_windows() {
        let events = vec![
            ev("a@x", "2026-05-12T10:00:00Z", "s1", 100, 50),
            ev("a@x", "2026-05-12T15:01:00Z", "s1", 200, 80), // 5h1m after start → new window
        ];
        let windows = compute_billing_windows(events);
        assert_eq!(windows.len(), 2);
    }

    #[test]
    fn inactivity_gap_starts_new_window() {
        let events = vec![
            ev("a@x", "2026-05-12T08:00:00Z", "s1", 100, 50),
            // 6-hour gap — rate limit resets
            ev("a@x", "2026-05-12T14:01:00Z", "s1", 200, 80),
        ];
        let windows = compute_billing_windows(events);
        assert_eq!(windows.len(), 2);
    }

    #[test]
    fn cross_session_same_window() {
        let events = vec![
            ev("a@x", "2026-05-12T10:00:00Z", "s1", 100, 50),
            ev("a@x", "2026-05-12T11:00:00Z", "s2", 200, 80), // different session, same window
        ];
        let windows = compute_billing_windows(events);
        assert_eq!(windows.len(), 1);
        assert_eq!(windows[0].session_count, 2);
    }

    #[test]
    fn two_users_independent_windows() {
        let events = vec![
            ev("a@x", "2026-05-12T10:00:00Z", "s1", 100, 50),
            ev("b@x", "2026-05-12T10:00:00Z", "s2", 200, 80),
        ];
        let windows = compute_billing_windows(events);
        assert_eq!(windows.len(), 2);
    }

    #[test]
    fn floor_to_hour_truncates_minutes() {
        let dt = parse_ts("2026-05-12T10:47:33Z").unwrap();
        let floored = floor_to_hour(dt);
        assert_eq!(floored, parse_ts("2026-05-12T10:00:00Z").unwrap());
    }

    #[test]
    fn window_start_is_floored_to_hour() {
        let events = vec![ev("a@x", "2026-05-12T10:47:00Z", "s1", 100, 50)];
        let windows = compute_billing_windows(events);
        assert_eq!(windows[0].start, parse_ts("2026-05-12T10:00:00Z").unwrap());
        assert_eq!(windows[0].end, parse_ts("2026-05-12T15:00:00Z").unwrap());
    }
}
