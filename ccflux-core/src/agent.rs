use std::sync::Arc;
use std::time::Duration;

/// Builds a ureq agent with bundled Mozilla root CAs (webpki-roots).
/// If CCFLUX_CA_CERT points to a PEM file, that CA is added on top of the Mozilla roots —
/// use this for testing with a self-signed or internal CA (e.g. Caddy local intermediate).
pub fn build(timeout_secs: u64, log: impl Fn(&str)) -> ureq::Agent {
    let builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(timeout_secs))
        .timeout_read(Duration::from_secs(timeout_secs));

    if let Ok(path) = std::env::var("CCFLUX_CA_CERT") {
        match std::fs::read(&path) {
            Err(e) => {
                log(&format!("tls: CCFLUX_CA_CERT read failed ({e}), using default TLS"));
            }
            Ok(pem) => {
                let mut roots = rustls::RootCertStore::empty();
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                let mut added = 0usize;
                for cert in rustls_pemfile::certs(&mut pem.as_slice()).flatten() {
                    match roots.add(cert) {
                        Ok(()) => added += 1,
                        Err(e) => log(&format!("tls: custom CA cert rejected: {e}")),
                    }
                }
                if added == 0 {
                    log(&format!("tls: CCFLUX_CA_CERT={path} — no certs added, using default TLS"));
                } else {
                    log(&format!("tls: custom CA loaded from {path} ({added} cert(s))"));
                    let provider = Arc::new(rustls::crypto::ring::default_provider());
                    let config = rustls::ClientConfig::builder_with_provider(provider)
                        .with_safe_default_protocol_versions()
                        .unwrap()
                        .with_root_certificates(roots)
                        .with_no_client_auth();
                    return builder.tls_config(Arc::new(config)).build();
                }
            }
        }
    }

    builder.build()
}
