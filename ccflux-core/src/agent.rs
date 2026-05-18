use std::sync::Arc;
use std::time::Duration;

/// Builds a ureq agent. Calls `log` with diagnostic messages at each step so the
/// caller can write them to the activity log and pinpoint any slowness.
///
/// Default: bundled Mozilla root CAs (webpki-roots) — instant, no system enumeration.
/// If CCFLUX_CA_CERT points to a PEM file, that CA is added on top of the Mozilla roots.
pub fn build(timeout_secs: u64, log: impl Fn(&str)) -> ureq::Agent {
    log("agent: creating builder");
    let builder = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(timeout_secs))
        .timeout_read(Duration::from_secs(timeout_secs));
    log("agent: builder created");

    let ca_cert_env = std::env::var("CCFLUX_CA_CERT");
    log(&format!("agent: CCFLUX_CA_CERT={ca_cert_env:?}"));

    if let Ok(path) = ca_cert_env {
        log(&format!("agent: reading PEM from {path}"));
        match std::fs::read(&path) {
            Err(e) => {
                log(&format!("agent: PEM read failed ({e}), using default TLS"));
            }
            Ok(pem) => {
                log("agent: PEM read ok, loading webpki roots");
                let mut roots = rustls::RootCertStore::empty();
                roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
                log("agent: webpki roots loaded, parsing custom CA");
                for cert in rustls_pemfile::certs(&mut pem.as_slice()).flatten() {
                    let _ = roots.add(cert);
                }
                log("agent: custom CA parsed, building ClientConfig");
                let provider = Arc::new(rustls::crypto::ring::default_provider());
                let config = rustls::ClientConfig::builder_with_provider(provider)
                    .with_safe_default_protocol_versions()
                    .unwrap()
                    .with_root_certificates(roots)
                    .with_no_client_auth();
                log("agent: calling tls_config().build()");
                let agent = builder.tls_config(Arc::new(config)).build();
                log("agent: done (custom TLS)");
                return agent;
            }
        }
    }

    log("agent: calling builder.build() (default webpki TLS)");
    let agent = builder.build();
    log("agent: done (default TLS)");
    agent
}
