use std::sync::OnceLock;

use base64ct::{Base64UrlUnpadded, Encoding};
use ed25519_dalek::SigningKey;
use sha2::{Digest, Sha256};

use crate::config::get_settings_dir;
use crate::crypto;

/// Writes `data` to `path`, restricting the result to its owner on unix.
///
/// The permission change belongs to writing rather than to the caller's umask: this file holds the
/// device's signing key, and a copy another account can read is enough to impersonate the device.
/// The result is returned so a caller that cannot continue without the file can say so.
pub(crate) fn write_key_file(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)?;
    #[cfg(unix)]
    {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

pub struct Fingerprint {
    pub value: String,
    pub public_key: String,
}

/// Identity a single run could not persist, kept so that same run stays self-consistent.
///
/// The key file is normally the only home of the identity, so without this a run that cannot write
/// it would mint a fresh key on every call: the sign-in and the heartbeat after it would report
/// different fingerprints and the server would reject the device it had just bound.
static UNPERSISTED_IDENTITY: OnceLock<SigningKey> = OnceLock::new();

/// Decodes the JSON payload stored in `device.key`.
///
/// The `pub` copy is cross-checked against the seed instead of trusted: a mismatch means the file is
/// corrupt or hand-assembled, and using such a key would silently move the device to a fingerprint
/// the server has never seen.
///
/// @return the signing key, or `None` when the payload is not a well-formed key of ours
pub(crate) fn decode_key_file(plain: &[u8]) -> Option<SigningKey> {
    let json = serde_json::from_slice::<serde_json::Value>(plain).ok()?;
    let seed = json.get("seed").and_then(|v| v.as_str())?;
    let public = json.get("pub").and_then(|v| v.as_str())?;
    let seed_bytes: [u8; 32] = Base64UrlUnpadded::decode_vec(seed).ok()?.try_into().ok()?;
    let public_bytes = Base64UrlUnpadded::decode_vec(public).ok()?;

    let key = SigningKey::from_bytes(&seed_bytes);
    (key.verifying_key().as_bytes() == public_bytes.as_slice()).then_some(key)
}

/// Whether `data` is a device key this client could start using.
///
/// Checked before an imported key replaces the one in place: an unreadable payload would leave the
/// machine holding an identity nothing can sign for, and every request would fail until the user
/// signed in again. Both storage generations are accepted — encrypted, and the legacy plaintext.
pub(crate) fn is_restorable_device_key(data: &[u8]) -> bool {
    let plain = crypto::decrypt(data).unwrap_or_else(|_| data.to_vec());
    decode_key_file(&plain).is_some()
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
        let key = decode_key_file(&plain);
        let should_migrate = key.is_some() && !was_encrypted;
        (key, should_migrate)
    } else {
        (None, false)
    };

    let signing_key = signing_key.unwrap_or_else(|| {
        // A key minted earlier in this run that could not be written to disk; reusing it is what
        // keeps every caller in this run on one identity.
        if let Some(cached) = UNPERSISTED_IDENTITY.get() {
            return cached.clone();
        }

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

        let Ok(encrypted) = crypto::encrypt(json.to_string().as_bytes()) else {
            // 加密不可用时宁可让私钥只活在内存里，也不写明文 seed —— 明文 device.key 能被本机
            // 任何进程读走，等于把设备身份交出去，比"本次启动后需要重新登录"严重得多。
            // 身份在本进程内由上面的缓存保持稳定，下次启动重新生成并再次尝试加密。
            crate::config::log_error(
                "fingerprint",
                "device key not persisted: encryption unavailable",
            );
            let _ = UNPERSISTED_IDENTITY.set(sk.clone());
            // 另一个线程可能已经抢先占用了缓存，以缓存里的那个为准，保证全进程同一身份。
            return UNPERSISTED_IDENTITY.get().cloned().unwrap_or(sk);
        };
        let _ = write_key_file(&key_path, &encrypted);
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
            let _ = write_key_file(&key_path, &encrypted);
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

#[cfg(test)]
mod tests {
    use super::{decode_key_file, is_restorable_device_key};
    use base64ct::{Base64UrlUnpadded, Encoding};
    use ed25519_dalek::SigningKey;

    /// 与 `get_or_create_fingerprint` 写出形状一致的 device.key 载荷。
    fn payload(sk: &SigningKey) -> Vec<u8> {
        serde_json::json!({
            "seed": Base64UrlUnpadded::encode_string(sk.as_bytes()),
            "pub": Base64UrlUnpadded::encode_string(sk.verifying_key().as_bytes()),
        })
        .to_string()
        .into_bytes()
    }

    #[test]
    fn device_key_payload_round_trips() {
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let decoded = decode_key_file(&payload(&sk)).expect("well-formed payload must decode");
        assert_eq!(decoded.to_bytes(), sk.to_bytes());
    }

    // 明文时代的 device.key 是 JSON，加密后的文件不是 JSON：两代存储都必须能识别，
    // 否则升级上来的安装会被当成没有身份、在第 71 行重新生成密钥并换掉指纹。
    #[test]
    fn encrypted_and_plaintext_payloads_are_both_restorable() {
        let sk = SigningKey::from_bytes(&[9u8; 32]);
        let plain = payload(&sk);
        assert!(is_restorable_device_key(&plain));
        assert!(is_restorable_device_key(
            &crate::crypto::encrypt(&plain).unwrap()
        ));
    }

    // pub 与 seed 不匹配的文件（写坏或被手工改过）不能被当成有效身份，
    // 否则设备会带着一个服务端从未登记过的指纹继续跑。
    #[test]
    fn mismatched_public_key_is_rejected() {
        let sk = SigningKey::from_bytes(&[1u8; 32]);
        let other = SigningKey::from_bytes(&[2u8; 32]);
        let mut json: serde_json::Value = serde_json::from_slice(&payload(&sk)).unwrap();
        json["pub"] = serde_json::Value::String(Base64UrlUnpadded::encode_string(
            other.verifying_key().as_bytes(),
        ));
        assert!(decode_key_file(json.to_string().as_bytes()).is_none());
    }

    #[test]
    fn malformed_payloads_are_rejected() {
        assert!(decode_key_file(b"").is_none());
        assert!(decode_key_file(b"not json").is_none());
        assert!(decode_key_file(br#"{"seed":"AAAA","pub":"AAAA"}"#).is_none());
        // 32 字节以外的 seed 不是 ed25519 私钥
        let short_seed = serde_json::json!({
            "seed": Base64UrlUnpadded::encode_string(&[0u8; 16]),
            "pub": Base64UrlUnpadded::encode_string(&[0u8; 32]),
        });
        assert!(decode_key_file(short_seed.to_string().as_bytes()).is_none());
        // 被截断的密文既解不开也不算明文 JSON
        assert!(!is_restorable_device_key(&[0x02, 1, 2, 3]));
    }
}
