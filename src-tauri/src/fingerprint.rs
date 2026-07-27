use base64ct::{Base64UrlUnpadded, Encoding};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

use crate::config::get_settings_dir;
use crate::crypto;

fn write_key_file(path: &std::path::Path, data: &[u8]) {
    let _ = std::fs::write(path, data);
    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
}

pub struct Fingerprint {
    pub value: String,
    pub public_key: String,
}

pub fn get_or_create_fingerprint() -> Fingerprint {
    let dir = get_settings_dir();
    std::fs::create_dir_all(&dir).ok();
    let key_path = dir.join("device.key");

    // Try encrypted format first, then fall back to legacy plaintext to preserve
    // existing fingerprints after the client upgraded from unencrypted storage.
    let (signing_key, should_migrate_to_encrypted) = if let Ok(data) = std::fs::read(&key_path) {
        let (plain, was_encrypted) = match crypto::decrypt(&data) {
            Ok(p) => (p, true),
            Err(_) => (data, false),
        };
        let key = if let Ok(json) = serde_json::from_slice::<serde_json::Value>(&plain) {
            if let (Some(seed), Some(pub_key_b64)) = (
                json.get("seed").and_then(|v| v.as_str()),
                json.get("pub").and_then(|v| v.as_str()),
            ) {
                if let (Ok(seed_bytes), Ok(pub_bytes)) = (
                    Base64UrlUnpadded::decode_vec(seed),
                    Base64UrlUnpadded::decode_vec(pub_key_b64),
                ) {
                    if seed_bytes.len() == 32 {
                        let sk: SigningKey =
                            SigningKey::from_bytes(&seed_bytes.try_into().unwrap());
                        let expected_pub = sk.verifying_key().to_bytes();
                        if expected_pub.as_ref() == pub_bytes.as_slice() {
                            Some(sk)
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let should_migrate = key.is_some() && !was_encrypted;
        (key, should_migrate)
    } else {
        (None, false)
    };

    let signing_key = signing_key.unwrap_or_else(|| {
        // Backup the existing key file before overwriting.
        if key_path.exists() {
            let backup = key_path.with_extension("key.bak");
            let _ = std::fs::copy(&key_path, &backup);
            #[cfg(unix)]
            {
                use std::fs;
                use std::os::unix::fs::PermissionsExt;
                let _ = fs::set_permissions(&backup, fs::Permissions::from_mode(0o600));
            }
        }

        use rand::rngs::OsRng;
        let mut csprng = OsRng;
        let sk = SigningKey::generate(&mut csprng);
        let vk = sk.verifying_key();

        let json = serde_json::json!({
            "seed": Base64UrlUnpadded::encode_string(sk.as_bytes()),
            "pub": Base64UrlUnpadded::encode_string(vk.as_bytes()),
        });

        let encrypted = crypto::encrypt(json.to_string().as_bytes())
            .unwrap_or_else(|_| json.to_string().into_bytes());
        write_key_file(&key_path, &encrypted);
        sk
    });

    // Migrate a legacy plaintext key file to encrypted storage once.
    if should_migrate_to_encrypted {
        let vk = signing_key.verifying_key();
        let json = serde_json::json!({
            "seed": Base64UrlUnpadded::encode_string(signing_key.as_bytes()),
            "pub": Base64UrlUnpadded::encode_string(vk.as_bytes()),
        });
        if let Ok(encrypted) = crypto::encrypt(json.to_string().as_bytes()) {
            write_key_file(&key_path, &encrypted);
        }
    }

    let public_key = signing_key.verifying_key();
    let pub_b64 = Base64UrlUnpadded::encode_string(public_key.as_bytes());

    let uuid = crate::platform::system_uuid();
    let hostname = crate::platform::hostname();
    let raw = format!("{}/{}/{}", uuid, hostname, pub_b64);

    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    Fingerprint {
        value: hash,
        public_key: pub_b64,
    }
}
