use aes_gcm::{
    aead::{Aead, KeyInit},
    Aes256Gcm, Nonce,
};
use argon2::Argon2;
use rand::RngCore;

const FORMAT_V2: u8 = 0x02;
const SALT_SIZE: usize = 16;
const NONCE_SIZE: usize = 12;

fn machine_secret() -> String {
    let uuid = crate::platform::system_uuid();
    let username = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "devops-user".to_string());
    format!("{}:{}", uuid, username)
}

fn derive_key(salt: &[u8]) -> Result<[u8; 32], String> {
    let secret = machine_secret();
    let mut okm = [0u8; 32];
    Argon2::default()
        .hash_password_into(secret.as_bytes(), salt, &mut okm)
        .map_err(|e| format!("Key derivation failed: {}", e))?;
    Ok(okm)
}

/// Encrypt plaintext bytes using AES-256-GCM with an Argon2id-derived machine-bound key.
pub fn encrypt(plaintext: &[u8]) -> Result<Vec<u8>, String> {
    let mut salt = [0u8; SALT_SIZE];
    rand::rngs::OsRng.fill_bytes(&mut salt);

    let key = derive_key(&salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| format!("Invalid key: {}", e))?;

    let mut nonce_bytes = [0u8; NONCE_SIZE];
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ciphertext = cipher
        .encrypt(nonce, plaintext)
        .map_err(|e| format!("Encryption failed: {}", e))?;

    let mut out = Vec::with_capacity(1 + SALT_SIZE + NONCE_SIZE + ciphertext.len());
    out.push(FORMAT_V2);
    out.extend_from_slice(&salt);
    out.extend_from_slice(&nonce_bytes);
    out.extend(&ciphertext);
    Ok(out)
}

/// Decrypt bytes previously encrypted by `encrypt`.
pub fn decrypt(data: &[u8]) -> Result<Vec<u8>, String> {
    if data.is_empty() {
        return Err("empty data".to_string());
    }
    if data[0] != FORMAT_V2 {
        return Err("unsupported format version".to_string());
    }
    if data.len() < 1 + SALT_SIZE + NONCE_SIZE {
        return Err("data too short".to_string());
    }

    let salt = &data[1..1 + SALT_SIZE];
    let nonce_bytes = &data[1 + SALT_SIZE..1 + SALT_SIZE + NONCE_SIZE];

    let key = derive_key(salt)?;
    let cipher = Aes256Gcm::new_from_slice(&key).map_err(|e| e.to_string())?;
    let nonce = Nonce::from_slice(nonce_bytes);

    cipher
        .decrypt(nonce, &data[1 + SALT_SIZE + NONCE_SIZE..])
        .map_err(|e| e.to_string())
}
