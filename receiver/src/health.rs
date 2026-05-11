use std::sync::atomic::{AtomicU64, Ordering};

use axum::{extract::State, http::StatusCode, response::IntoResponse};

use crate::{db, AppState};

pub struct Metrics {
    pub reports_accepted: AtomicU64,
    pub reports_auth_rejected: AtomicU64,
    pub reports_sig_rejected: AtomicU64,
    pub reports_rate_limited: AtomicU64,
    pub token_exchanges: AtomicU64,
    pub key_registrations: AtomicU64,
}

impl Metrics {
    pub fn new() -> Self {
        Self {
            reports_accepted: AtomicU64::new(0),
            reports_auth_rejected: AtomicU64::new(0),
            reports_sig_rejected: AtomicU64::new(0),
            reports_rate_limited: AtomicU64::new(0),
            token_exchanges: AtomicU64::new(0),
            key_registrations: AtomicU64::new(0),
        }
    }

    pub fn inc(&self, counter: &AtomicU64) {
        counter.fetch_add(1, Ordering::Relaxed);
    }
}

pub async fn handle_health(State(state): State<AppState>) -> impl IntoResponse {
    match db::ping(&state.pool).await {
        Ok(()) => (StatusCode::OK, r#"{"status":"ok","db":"ok"}"#),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, r#"{"status":"degraded","db":"error"}"#),
    }
}

pub async fn handle_metrics(State(state): State<AppState>) -> impl IntoResponse {
    let active_tokens = db::count_active_access_tokens(&state.pool)
        .await
        .unwrap_or(0);

    let m = &state.metrics;
    let body = format!(
        "\
# HELP ccflux_reports_accepted_total Usage reports accepted (HTTP 200)
# TYPE ccflux_reports_accepted_total counter
ccflux_reports_accepted_total {reports_accepted}
# HELP ccflux_reports_auth_rejected_total Usage reports rejected due to invalid/expired token
# TYPE ccflux_reports_auth_rejected_total counter
ccflux_reports_auth_rejected_total {reports_auth_rejected}
# HELP ccflux_reports_sig_rejected_total Usage reports rejected due to signature failure
# TYPE ccflux_reports_sig_rejected_total counter
ccflux_reports_sig_rejected_total {reports_sig_rejected}
# HELP ccflux_reports_rate_limited_total Usage reports dropped due to rate limiting
# TYPE ccflux_reports_rate_limited_total counter
ccflux_reports_rate_limited_total {reports_rate_limited}
# HELP ccflux_token_exchanges_total Successful refresh→access token exchanges
# TYPE ccflux_token_exchanges_total counter
ccflux_token_exchanges_total {token_exchanges}
# HELP ccflux_key_registrations_total Successful device key registrations
# TYPE ccflux_key_registrations_total counter
ccflux_key_registrations_total {key_registrations}
# HELP ccflux_active_access_tokens Current number of non-expired access tokens
# TYPE ccflux_active_access_tokens gauge
ccflux_active_access_tokens {active_tokens}
",
        reports_accepted = m.reports_accepted.load(Ordering::Relaxed),
        reports_auth_rejected = m.reports_auth_rejected.load(Ordering::Relaxed),
        reports_sig_rejected = m.reports_sig_rejected.load(Ordering::Relaxed),
        reports_rate_limited = m.reports_rate_limited.load(Ordering::Relaxed),
        token_exchanges = m.token_exchanges.load(Ordering::Relaxed),
        key_registrations = m.key_registrations.load(Ordering::Relaxed),
        active_tokens = active_tokens,
    );

    (
        StatusCode::OK,
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        body,
    )
}
