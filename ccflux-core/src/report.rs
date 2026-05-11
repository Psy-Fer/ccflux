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
pub fn post(endpoint: &str, token: &str, body: &str, key: &DeviceKey) -> ReportStatus {
    if !endpoint.starts_with("https://") {
        return ReportStatus::Failed(format!(
            "endpoint must use https:// — plain HTTP would expose the bearer token; got: {endpoint}"
        ));
    }

    let timestamp = chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let signature = key.sign(body.as_bytes(), &timestamp);
    let sig_header = format!("ed25519 {} {}", signature, key.public_key_b64());

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(5))
        .build();

    match agent
        .post(endpoint)
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
            other => ReportStatus::Failed(format!("403: {other}")),
        },
        Err(ureq::Error::Status(status, _)) => ReportStatus::Failed(format!("HTTP {status}")),
        Err(e) => ReportStatus::Failed(e.to_string()),
    }
}
