use std::path::{Path, PathBuf};

use base64::{engine::general_purpose::STANDARD, Engine};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand::rngs::OsRng;

use crate::offset;

pub struct DeviceKey {
    signing_key: SigningKey,
}

impl DeviceKey {
    fn generate() -> Self {
        Self { signing_key: SigningKey::generate(&mut OsRng) }
    }

    fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { signing_key: SigningKey::from_bytes(&bytes) }
    }

    pub fn public_key_b64(&self) -> String {
        STANDARD.encode(VerifyingKey::from(&self.signing_key).to_bytes())
    }

    /// Signs `body_bytes ++ '\n' ++ timestamp` and returns the base64-encoded signature.
    pub fn sign(&self, body: &[u8], timestamp: &str) -> String {
        let mut msg = body.to_vec();
        msg.push(b'\n');
        msg.extend_from_slice(timestamp.as_bytes());
        STANDARD.encode(self.signing_key.sign(&msg).to_bytes())
    }
}

/// Loads the private key from disk, generating and saving it if absent.
pub fn load_or_generate(data_dir: &Path) -> DeviceKey {
    let path = signing_key_path(data_dir);
    if let Ok(bytes) = std::fs::read(&path) {
        if let Ok(arr) = bytes.try_into() {
            return DeviceKey::from_bytes(arr);
        }
    }
    let key = DeviceKey::generate();
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, key.signing_key.to_bytes());
    offset::set_secure_permissions(&path);
    // New key means any prior registration is stale.
    let _ = std::fs::remove_file(key_registered_path(data_dir));
    key
}

pub fn is_registered(data_dir: &Path) -> bool {
    let reg_path = key_registered_path(data_dir);
    if let Ok(stored) = std::fs::read_to_string(&reg_path) {
        // Sanity check: stored pubkey must match the current signing key.
        if let Ok(key_bytes) = std::fs::read(signing_key_path(data_dir)) {
            if let Ok(arr) = key_bytes.try_into() {
                let current = DeviceKey::from_bytes(arr).public_key_b64();
                return stored.trim() == current;
            }
        }
    }
    false
}

pub fn mark_registered(data_dir: &Path, pubkey_b64: &str) {
    let path = key_registered_path(data_dir);
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, pubkey_b64);
}

pub fn is_revoked(data_dir: &Path) -> bool {
    key_revoked_path(data_dir).exists()
}

pub fn mark_revoked(data_dir: &Path) {
    let path = key_revoked_path(data_dir);
    let _ = std::fs::create_dir_all(path.parent().unwrap());
    let _ = std::fs::write(&path, b"");
}

/// Attempts to register the public key with the receiver.
/// Returns true on success (200 or 409 already-registered), false on any failure.
pub fn try_register(
    data_dir: &Path,
    report_endpoint: &str,
    access_token: &str,
    key: &DeviceKey,
) -> bool {
    let url = register_key_endpoint(report_endpoint);
    let pubkey = key.public_key_b64();
    let device_id = get_device_id();

    let body = format!(
        r#"{{"public_key":"{pubkey}","device_id":"{device_id}"}}"#
    );

    let agent = ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(5))
        .timeout_read(std::time::Duration::from_secs(5))
        .build();

    match agent
        .post(&url)
        .set("Authorization", &format!("Bearer {access_token}"))
        .set("Content-Type", "application/json")
        .send_string(&body)
    {
        Ok(_) => {
            mark_registered(data_dir, &pubkey);
            true
        }
        Err(ureq::Error::Status(409, _)) => {
            // Already registered (idempotent).
            mark_registered(data_dir, &pubkey);
            true
        }
        Err(_) => false,
    }
}

fn register_key_endpoint(report_endpoint: &str) -> String {
    match report_endpoint.rfind('/') {
        Some(i) => format!("{}/register-key", &report_endpoint[..i]),
        None => format!("{report_endpoint}/register-key"),
    }
}

fn get_device_id() -> String {
    std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

fn signing_key_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ccflux").join("signing_key")
}

fn key_registered_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ccflux").join("key_registered")
}

fn key_revoked_path(data_dir: &Path) -> PathBuf {
    data_dir.join("ccflux").join("key_revoked")
}
