use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

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
    pub password: String,
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

/// 追加写入错误日志（Windows 无控制台，配置保存/加载失败需落盘才能排查）。
pub fn log_error(context: &str, detail: &str) {
    let dir = get_settings_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("error.log");
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        use std::io::Write;
        let _ = writeln!(f, "[{}] {}: {}", secs, context, detail);
    }
}

pub fn load_config() -> Option<Config> {
    let path = get_settings_dir().join("config.json");
    let encrypted = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            log_error("load_config", &format!("read access denied, repairing ACL: {}", e));
            repair_file_acl(&path);
            match std::fs::read(&path) {
                Ok(b) => b,
                Err(e2) => {
                    log_error("load_config", &format!("read failed after ACL repair: {}", e2));
                    return None;
                }
            }
        }
        Err(e) => {
            log_error("load_config", &format!("read failed: {}", e));
            return None;
        }
    };
    let data = match crate::crypto::decrypt(&encrypted) {
        Ok(d) => d,
        Err(e) => {
            log_error("load_config", &format!("decrypt failed: {}", e));
            return None;
        }
    };
    match serde_json::from_slice(&data) {
        Ok(c) => Some(c),
        Err(e) => {
            log_error("load_config", &format!("json parse failed: {}", e));
            None
        }
    }
}

pub fn save_config(config: &Config) -> Result<(), String> {
    let dir = get_settings_dir();
    std::fs::create_dir_all(&dir).map_err(|e| {
        let m = format!("Failed to create config dir: {}", e);
        log_error("save_config", &m);
        m
    })?;
    restrict_dir_permissions(&dir).map_err(|e| {
        log_error("save_config", &e);
        e
    })?;

    // Validate language if set
    if !config.language.is_empty() && !SUPPORTED_LANGUAGES.contains(&config.language.as_str()) {
        let m = format!(
            "Unsupported language '{}'. Supported: {:?}",
            config.language, SUPPORTED_LANGUAGES
        );
        log_error("save_config", &m);
        return Err(m);
    }

    let data = serde_json::to_vec(config).map_err(|e| {
        let m = format!("Failed to serialize config: {}", e);
        log_error("save_config", &m);
        m
    })?;
    let encrypted = crate::crypto::encrypt(&data).map_err(|e| {
        log_error("save_config", &e);
        e
    })?;
    let path = dir.join("config.json");
    if let Err(e) = std::fs::write(&path, &encrypted) {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            log_error("save_config", &format!("write access denied, repairing ACL: {}", e));
            repair_file_acl(&path);
            std::fs::write(&path, &encrypted).map_err(|e2| {
                let m = format!("Failed to write config: {}", e2);
                log_error("save_config", &m);
                m
            })?;
        } else {
            let m = format!("Failed to write config: {}", e);
            log_error("save_config", &m);
            return Err(m);
        }
    }
    restrict_file_permissions(&path).map_err(|e| {
        log_error("save_config", &e);
        e
    })?;
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
fn restrict_file_permissions(_path: &std::path::Path) -> Result<(), String> {
    // 不做 ACL 收紧：icacls /inheritance:r /grant:r 会剥离当前用户对文件的继承权限，
    // 一旦 USERNAME 与登录账户不一致，后续读写即报"拒绝访问 (os error 5)"。
    // 文件位于用户私有目录且内容已加密，依赖默认 ACL 即可。
    Ok(())
}

/// 恢复文件继承的 ACL（用于修复旧版本 icacls 剥离权限导致的"拒绝访问"）。
#[cfg(target_os = "windows")]
fn repair_file_acl(path: &std::path::Path) {
    use std::os::windows::process::CommandExt;
    use std::process::Command;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let _ = Command::new("icacls")
        .arg(path.to_string_lossy().into_owned())
        .arg("/reset")
        .creation_flags(CREATE_NO_WINDOW)
        .output();
}

#[cfg(not(target_os = "windows"))]
fn repair_file_acl(_path: &std::path::Path) {}

#[cfg(not(any(unix, target_os = "windows")))]
fn restrict_file_permissions(_path: &std::path::Path) -> Result<(), String> {
    Ok(())
}
