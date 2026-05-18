use std::sync::Arc;
use std::time::Duration;

/// Builds a ureq agent with the bundled Mozilla root CAs.
/// If CCFLUX_CA_CERT points to a PEM file, that CA is added on top of the
/// Mozilla roots — use this for testing with a self-signed CA (e.g. Caddy local CA).
pub fn build(timeout_secs: u64) -> ureq::Agent {
    let builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(timeout_secs))
        .timeout_read(Duration::from_secs(timeout_secs));

    if let Ok(path) = std::env::var("CCFLUX_CA_CERT") {
        if let Ok(pem) = std::fs::read(&path) {
            let mut roots = rustls::RootCertStore::empty();
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            for cert in rustls_pemfile::certs(&mut pem.as_slice()).flatten() {
                let _ = roots.add(cert);
            }
            let config = rustls::ClientConfig::builder()
                .with_root_certificates(roots)
                .with_no_client_auth();
            return builder.tls_config(Arc::new(config)).build();
        }
    }

    builder.build()
}
