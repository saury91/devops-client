use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use rand::RngCore;
use sha2::{Digest, Sha256};

const FORMAT_V1: u8 = 0x01;
const NONCE_SIZE: usize = 12;

fn derive_key() -> [u8; 32] {
    let uuid = machine_uuid();
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_default();
    let salt = format!("{}:{}", uuid, username);
    let mut hasher = Sha256::new();
    hasher.update(salt.as_bytes());
    hasher.finalize().into()
}

fn machine_uuid() -> String {
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
        if let Ok(o) = std::process::Command::new("reg")
            .args([
                "query",
                r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Cryptography",
                "/v",
                "MachineGuid",
            ])
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
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
            let id = id.trim().to_string();
            if !id.is_empty() && id.len() >= 32 {
                return id;
            }
        }
    }

    "unknown".to_string()
}

/// Encrypt plaintext bytes using AES-256-GCM with a machine-bound key.
pub fn encrypt(plaintext: &[u8]) -> Vec<u8> {
    let key = derive_key();
    let cipher = Aes256Gcm::new_from_slice(&key).expect("valid 256-bit key");
    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let mut ciphertext = cipher
        .encrypt(nonce, plaintext)
        .expect("encryption succeeded");

    let mut out = Vec::with_capacity(1 + NONCE_SIZE + ciphertext.len());
    out.push(FORMAT_V1);
    out.extend_from_slice(&nonce_bytes);
    out.append(&mut ciphertext);
    out
}

/// Decrypt bytes previously encrypted by `encrypt`.
pub fn decrypt(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.is_empty() {
        return Err("empty data".to_string());
    }
    if data[0] != FORMAT_V1 {
        return Err("unsupported format version".to_string());
    }
    if data.len() < 1 + NONCE_SIZE {
        return Err("data too short".to_string());
    }

    let key = derive_key();
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(&data[1..1 + NONCE_SIZE]);

    cipher
        .decrypt(nonce, &data[1 + NONCE_SIZE..])
        .map_err(|e| e.to_string())
}
