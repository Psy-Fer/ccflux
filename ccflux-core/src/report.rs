use std::time::Duration;

use crate::model::UsagePayload;

pub fn post(endpoint: &str, token: &str, payload: &UsagePayload) -> Result<(), String> {
    if !endpoint.starts_with("https://") {
        return Err(format!(
            "endpoint must use https:// — plain HTTP would expose the bearer token; got: {endpoint}"
        ));
    }

    let body = serde_json::to_string(payload).map_err(|e| format!("serialize: {e}"))?;

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(5))
        .build();

    let resp = agent
        .post(endpoint)
        .set("Authorization", &format!("Bearer {token}"))
        .set("Content-Type", "application/json")
        .send_string(&body)
        .map_err(|e| format!("POST {endpoint}: {e}"))?;

    let status = resp.status();
    if status >= 300 {
        let body = resp.into_string().unwrap_or_default();
        return Err(format!("receiver returned HTTP {status}: {body}"));
    }

    Ok(())
}
