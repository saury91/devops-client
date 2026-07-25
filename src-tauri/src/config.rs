use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub server_url: String,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub login_at: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub nickname: String,
}

pub fn get_settings_dir() -> PathBuf {
    dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".devops-client")
}

pub fn load_config() -> Option<Config> {
    let path = get_settings_dir().join("config.json");
    let encrypted = std::fs::read(&path).ok()?;
    let data = crate::crypto::decrypt(&encrypted).ok()?;
    serde_json::from_slice(&data).ok()
}

pub fn save_config(config: &Config) {
    let dir = get_settings_dir();
    std::fs::create_dir_all(&dir).ok();
    let data = serde_json::to_vec(config).unwrap_or_default();
    let encrypted = crate::crypto::encrypt(&data);
    std::fs::write(dir.join("config.json"), encrypted).ok();
}
