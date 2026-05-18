use std::sync::Arc;
use std::time::Duration;

pub fn build(timeout_secs: u64) -> ureq::Agent {
    let mut roots = rustls::RootCertStore::empty();
    match rustls_native_certs::load_native_certs() {
        Ok(certs) => {
            for c in certs {
                let _ = roots.add(c);
            }
        }
        Err(_) => {}
    }
    let config = rustls::ClientConfig::builder()
        .with_root_certificates(roots)
        .with_no_client_auth();
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(timeout_secs))
        .timeout_read(Duration::from_secs(timeout_secs))
        .tls_config(Arc::new(config))
        .build()
}
