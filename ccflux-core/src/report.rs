use std::time::Duration;

use crate::signing::DeviceKey;

pub enum ReportStatus {
    Accepted,
    KeyRevoked,
    TimestampStale,
    SignatureInvalid,
    KeyNotRegistered,
    Failed(String),
}

/// Signs and POSTs a JSON payload string. Used for both live reports and queue drains.
/// Strips control characters from a server-supplied header value before logging.
fn sanitize_header(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii() && !c.is_ascii_control())
        .take(64)
        .collect()
}

/// Derives the /report URL from the configured endpoint.
/// Accepts either a base URL ("https://host") or a full report URL ("https://host/report").
fn report_url(endpoint: &str) -> String {
    if endpoint.matches('/').count() > 2 {
        endpoint.to_string()
    } else {
        format!("{endpoint}/report")
    }
}

pub fn post(endpoint: &str, token: &str, body: &str, key: &DeviceKey) -> ReportStatus {
    let allow_http = std::env::var("CCFLUX_ALLOW_HTTP").as_deref() == Ok("1");
    if !allow_http && !endpoint.starts_with("https://") {
        return ReportStatus::Failed(format!(
            "endpoint must use https:// — plain HTTP would expose the bearer token; got: {endpoint}"
        ));
    }

    let url = report_url(endpoint);
    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let signature = key.sign(body.as_bytes(), &timestamp);
    let sig_header = format!("ed25519 {} {}", signature, key.public_key_b64());

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(5))
        .build();

    match agent
        .post(&url)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .set("X-CCFLUX-Signature", &sig_header)
        .set("X-CCFLUX-Timestamp", &timestamp)
        .send_string(body)
    {
        Ok(_) => ReportStatus::Accepted,
        Err(ureq::Error::Status(403, resp)) => match resp.header("x-ccflux-error").unwrap_or("") {
            "key-revoked" => ReportStatus::KeyRevoked,
            "timestamp-stale" => ReportStatus::TimestampStale,
            "signature-invalid" => ReportStatus::SignatureInvalid,
            "key-not-registered" => ReportStatus::KeyNotRegistered,
            other => ReportStatus::Failed(format!("403: {}", sanitize_header(other))),
        },
        Err(ureq::Error::Status(status, _)) => ReportStatus::Failed(format!("HTTP {status}")),
        Err(e) => ReportStatus::Failed(e.to_string()),
    }
}
