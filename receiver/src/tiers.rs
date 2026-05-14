use std::collections::HashMap;

use crate::billing::BillingWindow;

#[derive(Clone, Debug)]
pub struct TierClassification {
    pub label: String,
    pub peak_tokens: Option<i64>,
    /// "inferred" | "limit_hit"
    pub method: String,
    /// "none" | "low" | "medium" | "high"
    pub confidence: String,
    pub window_count: usize,
}

impl TierClassification {
    pub fn unknown() -> Self {
        Self {
            label: "Unknown".to_string(),
            peak_tokens: None,
            method: "inferred".to_string(),
            confidence: "none".to_string(),
            window_count: 0,
        }
    }
}

pub fn confidence_from(method: &str, window_count: usize) -> String {
    if method == "limit_hit" {
        "high".to_string()
    } else if window_count >= 10 {
        "medium".to_string()
    } else if window_count >= 3 {
        "low".to_string()
    } else {
        "none".to_string()
    }
}

/// Minimum completed windows before a user is classified (not Unknown).
const MIN_WINDOWS: usize = 3;

/// Consecutive peak ratio above which a new tier boundary is declared.
/// 1.8× means a jump from e.g. 90 k → 162 k+ is a tier change.
const TIER_GAP_RATIO: f64 = 1.8;

/// Infers tier classifications for all users from their completed billing windows.
///
/// Algorithm:
///  1. Collect all completed window peaks per user (active windows excluded).
///  2. Per user: sort peaks, take 75th-percentile value as representative.
///     This drops one-off outliers caused by Anthropic inactivity resets.
///  3. Users with < MIN_WINDOWS completed windows → Unknown (not enough signal).
///  4. Sort remaining users by representative peak ascending.
///  5. Walk the sorted list; start a new tier when consecutive peaks differ by
///     more than TIER_GAP_RATIO. Assign Tier 1, Tier 2, ... bottom-up.
pub fn infer_tiers(windows: &[BillingWindow]) -> HashMap<String, TierClassification> {
    let mut user_peaks: HashMap<String, Vec<i64>> = HashMap::new();
    for w in windows {
        if !w.is_active {
            let peak = w.billed_tokens();
            if peak > 0 {
                user_peaks
                    .entry(w.user_email.clone())
                    .or_default()
                    .push(peak);
            }
        }
    }

    let mut classifiable: Vec<(String, i64, usize)> = Vec::new();
    let mut insufficient: Vec<(String, usize)> = Vec::new();

    for (email, mut peaks) in user_peaks {
        let n = peaks.len();
        if n < MIN_WINDOWS {
            insufficient.push((email, n));
            continue;
        }
        peaks.sort_unstable();
        let idx = (n * 75 / 100).min(n - 1);
        classifiable.push((email, peaks[idx], n));
    }

    // Sort ascending by representative peak so we can walk and detect gaps.
    classifiable.sort_by_key(|(_, rep, _)| *rep);

    let mut result: HashMap<String, TierClassification> = HashMap::new();
    let mut tier_num = 1usize;
    let mut prev_rep: Option<i64> = None;

    for (email, rep, n) in &classifiable {
        if let Some(prev) = prev_rep {
            if prev > 0 && (*rep as f64 / prev as f64) > TIER_GAP_RATIO {
                tier_num += 1;
            }
        }
        result.insert(
            email.clone(),
            TierClassification {
                label: format!("Tier {tier_num}"),
                peak_tokens: Some(*rep),
                method: "inferred".to_string(),
                confidence: confidence_from("inferred", *n),
                window_count: *n,
            },
        );
        prev_rep = Some(*rep);
    }

    for (email, n) in insufficient {
        result.insert(
            email,
            TierClassification {
                label: "Unknown".to_string(),
                peak_tokens: None,
                method: "inferred".to_string(),
                confidence: "none".to_string(),
                window_count: n,
            },
        );
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn window(email: &str, billed: i64, active: bool) -> BillingWindow {
        let now = Utc::now();
        BillingWindow {
            user_email: email.to_string(),
            start: now,
            end: now,
            is_active: active,
            input_tokens: billed,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_write_tokens: 0,
            turns: 1,
            session_count: 1,
            last_entry: now,
        }
    }

    fn make_windows(email: &str, peaks: &[i64]) -> Vec<BillingWindow> {
        peaks.iter().map(|&p| window(email, p, false)).collect()
    }

    #[test]
    fn fewer_than_min_windows_is_unknown() {
        let ws = make_windows("u@x.com", &[50_000, 60_000]);
        let tiers = infer_tiers(&ws);
        assert_eq!(tiers["u@x.com"].label, "Unknown");
        assert_eq!(tiers["u@x.com"].confidence, "none");
    }

    #[test]
    fn active_windows_are_excluded() {
        let mut ws = make_windows("u@x.com", &[50_000, 60_000, 55_000]);
        ws.push(window("u@x.com", 999_999, true)); // active, should be excluded
        let tiers = infer_tiers(&ws);
        // 3 completed windows → classifiable, active one doesn't inflate peak
        assert_eq!(tiers["u@x.com"].label, "Tier 1");
        assert!(tiers["u@x.com"].peak_tokens.unwrap() < 999_999);
    }

    #[test]
    fn two_well_separated_users_get_different_tiers() {
        let mut ws = make_windows("low@x.com", &[10_000, 12_000, 11_000]);
        ws.extend(make_windows("high@x.com", &[90_000, 95_000, 92_000]));
        let tiers = infer_tiers(&ws);
        let low = tiers["low@x.com"].label.clone();
        let high = tiers["high@x.com"].label.clone();
        assert_ne!(low, high);
        assert_eq!(low, "Tier 1");
        assert_eq!(high, "Tier 2");
    }

    #[test]
    fn users_within_ratio_share_a_tier() {
        // 50k and 80k: ratio 1.6 < 1.8, same tier
        let mut ws = make_windows("a@x.com", &[50_000, 52_000, 51_000]);
        ws.extend(make_windows("b@x.com", &[78_000, 80_000, 79_000]));
        let tiers = infer_tiers(&ws);
        assert_eq!(tiers["a@x.com"].label, tiers["b@x.com"].label);
    }

    #[test]
    fn outlier_peaks_are_dampened_by_percentile() {
        // User has 9 normal windows at ~50k and one anomalous 500k (reset outlier).
        let mut peaks: Vec<i64> = vec![50_000; 9];
        peaks.push(500_000);
        let ws = make_windows("u@x.com", &peaks);
        let tiers = infer_tiers(&ws);
        // 75th percentile of 10 values = index 7 = 50_000 (sorted, idx 7 of 10)
        assert!(tiers["u@x.com"].peak_tokens.unwrap() < 100_000);
    }

    #[test]
    fn three_tier_org() {
        let mut ws = make_windows("tier1@x.com", &[10_000, 11_000, 10_500]);
        ws.extend(make_windows("tier2@x.com", &[90_000, 92_000, 91_000]));
        ws.extend(make_windows("tier3@x.com", &[500_000, 510_000, 505_000]));
        let tiers = infer_tiers(&ws);
        assert_eq!(tiers["tier1@x.com"].label, "Tier 1");
        assert_eq!(tiers["tier2@x.com"].label, "Tier 2");
        assert_eq!(tiers["tier3@x.com"].label, "Tier 3");
    }
}
