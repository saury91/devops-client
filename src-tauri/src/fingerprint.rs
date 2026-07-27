use base64ct::{Base64UrlUnpadded, Encoding};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

use crate::config::get_settings_dir;

pub struct Fingerprint {
    pub value: String,
    pub public_key: String,
}

fn get_system_uuid() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(o) = std::process::Command::new("sh")
            .args([
                "-c",
                "system_profiler SPHardwareDataType 2>/dev/null | awk '/Hardware UUID/{print $NF}'",
            ])
            .output()
        {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !s.is_empty() && s != "unknown" {
                return s;
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        // Try MachineGuid first (stable per OS install)
        if let Ok(o) = std::process::Command::new("reg")
            .args([
                "query",
                r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Cryptography",
                "/v",
                "MachineGuid",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            let s = String::from_utf8_lossy(&o.stdout);
            if let Some(line) = s.lines().find(|l| l.contains("REG_SZ")) {
                let guid = line.split("REG_SZ").nth(1).unwrap_or("").trim().to_string();
                if !guid.is_empty() {
                    return guid;
                }
            }
        }
        // Fallback to wmic
        if let Ok(o) = std::process::Command::new("wmic")
            .args(["csproduct", "get", "uuid"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            let s = String::from_utf8_lossy(&o.stdout);
            let uuid = s.lines().nth(1).unwrap_or("").trim().to_string();
            if !uuid.is_empty() {
                return uuid;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // /etc/machine-id is the most stable identifier on systemd Linux
        if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
            let id = id.trim().to_string();
            if !id.is_empty() && id.len() >= 32 {
                return id;
            }
        }
        // Fallback: DMI product UUID
        if let Ok(o) = std::process::Command::new("sh")
            .args(["-c", "cat /sys/class/dmi/id/product_uuid 2>/dev/null || dmidecode -s system-uuid 2>/dev/null"])
            .output()
        {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !s.is_empty() { return s; }
        }
    }

    "unknown".to_string()
}

pub fn get_or_create_fingerprint() -> Fingerprint {
    let dir = get_settings_dir();
    std::fs::create_dir_all(&dir).ok();
    let key_path = dir.join("device.key");

    // Try encrypted format first, then fall back to legacy plaintext to preserve
    // existing fingerprints after the client upgraded from unencrypted storage.
    let (signing_key, should_migrate_to_encrypted) = if let Ok(data) = std::fs::read(&key_path) {
        let (plain, was_encrypted) = match crate::crypto::decrypt(&data) {
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
        // Backup the existing key file before overwriting, in case it was an
        // encrypted file that can no longer be decrypted (machine key changed).
        if key_path.exists() {
            let _ = std::fs::copy(&key_path, key_path.with_extension("key.bak"));
        }

        use rand::rngs::OsRng;
        let mut csprng = OsRng;
        let sk = SigningKey::generate(&mut csprng);
        let vk = sk.verifying_key();

        let json = serde_json::json!({
            "seed": Base64UrlUnpadded::encode_string(sk.as_bytes()),
            "pub": Base64UrlUnpadded::encode_string(vk.as_bytes()),
        });

        let encrypted = crate::crypto::encrypt(json.to_string().as_bytes());
        std::fs::write(&key_path, encrypted).ok();
        sk
    });

    // Migrate a legacy plaintext key file to encrypted storage once.
    if should_migrate_to_encrypted {
        let vk = signing_key.verifying_key();
        let json = serde_json::json!({
            "seed": Base64UrlUnpadded::encode_string(signing_key.as_bytes()),
            "pub": Base64UrlUnpadded::encode_string(vk.as_bytes()),
        });
        let encrypted = crate::crypto::encrypt(json.to_string().as_bytes());
        std::fs::write(&key_path, encrypted).ok();
    }

    let public_key = signing_key.verifying_key();
    let pub_b64 = Base64UrlUnpadded::encode_string(public_key.as_bytes());

    let uuid = get_system_uuid();
    let raw = format!("{}/{}", uuid, pub_b64);

    let mut hasher = Sha256::new();
    hasher.update(raw.as_bytes());
    let hash = format!("{:x}", hasher.finalize());

    Fingerprint {
        value: hash,
        public_key: pub_b64,
    }
}
