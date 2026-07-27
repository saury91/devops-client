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
    #[serde(default)]
    pub language: String,
}

const SUPPORTED_LANGUAGES: &[&str] = &["zh", "en"];

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

pub fn save_config(config: &Config) -> Result<(), String> {
    let dir = get_settings_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create config dir: {}", e))?;
    restrict_dir_permissions(&dir)?;

    // Validate language if set
    if !config.language.is_empty() && !SUPPORTED_LANGUAGES.contains(&config.language.as_str()) {
        return Err(format!(
            "Unsupported language '{}'. Supported: {:?}",
            config.language, SUPPORTED_LANGUAGES
        ));
    }

    let data =
        serde_json::to_vec(config).map_err(|e| format!("Failed to serialize config: {}", e))?;
    let encrypted = crate::crypto::encrypt(&data)?;
    let path = dir.join("config.json");
    std::fs::write(&path, encrypted).map_err(|e| format!("Failed to write config: {}", e))?;
    restrict_file_permissions(&path)?;
    Ok(())
}

#[cfg(unix)]
fn restrict_dir_permissions(path: &std::path::Path) -> Result<(), String> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|e| format!("Failed to set dir permissions: {}", e))
}

#[cfg(not(unix))]
fn restrict_dir_permissions(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &std::path::Path) -> Result<(), String> {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Failed to set file permissions: {}", e))
}

#[cfg(target_os = "windows")]
fn restrict_file_permissions(path: &std::path::Path) -> Result<(), String> {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let path_str = path.to_string_lossy().into_owned();
    let perm_str = std::env::var("USERNAME").unwrap_or_else(|_| "User".to_string()) + ":(R,W)";
    let _ = Command::new("icacls")
        .args([
            path_str.as_str(),
            "/inheritance:r",
            "/grant:r",
            perm_str.as_str(),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    Ok(())
}

#[cfg(not(any(unix, target_os = "windows")))]
fn restrict_file_permissions(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}
