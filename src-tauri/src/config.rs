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

/// 单个日志文件的上限（1 MiB）。超过就滚动为 `error.log.1`，只保留一代。
///
/// 不设上限会真的出问题：心跳每 30 秒一次、每次失败都写一行，一次服务端故障就能在
/// 几小时内把这个文件写到几百兆。用户目录被悄悄占满，比丢几条日志更难排查；而保留
/// 多代又会让「导出日志」的附件翻倍，所以只留一代。
const LOG_MAX_BYTES: u64 = 1024 * 1024;

/// 把超限的日志滚动到 `error.log.1`（覆盖上一代）。
fn rotate_log_if_needed(path: &std::path::Path) {
    let oversized = std::fs::metadata(path)
        .map(|m| m.len() >= LOG_MAX_BYTES)
        .unwrap_or(false);
    if !oversized {
        return;
    }
    let backup = path.with_extension("log.1");
    // Windows 上 rename 到已存在的目标会失败，先删掉旧备份。
    let _ = std::fs::remove_file(&backup);
    let _ = std::fs::rename(path, &backup);
}

/// 追加写入错误日志（Windows 无控制台，配置保存/加载失败需落盘才能排查）。
pub fn log_error(context: &str, detail: &str) {
    let dir = get_settings_dir();
    if std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("error.log");
    rotate_log_if_needed(&path);
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        use std::io::Write;
        let _ = writeln!(f, "[{}] {}: {}", secs, context, detail);
    }
}

/// 安装 panic hook，把 panic 的位置与消息写进 `error.log`。
///
/// Windows 上没有控制台（`main.rs` 首行即 `windows_subsystem = "windows"`），而主线程
/// 之外的 panic 既不会结束进程、也不会打印到任何地方：心跳线程、代理线程、工作台票据签发路径上
/// 的一次 panic 只会表现为「某个功能悄悄不工作了」，现场还无法复现。把位置和消息落盘，
/// 才能靠「导出日志」把它带回来。
///
/// 同时保留默认 hook —— 调试构建里 stderr 的 panic 输出仍然照常。
pub fn install_panic_hook() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".to_string());
        // payload 通常是 `&str` 或 `String`；`panic_any` 传进来的其它类型没有可读的
        // 文本形式，记一个占位符即可，不必去 downcast 它。
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string payload>".to_string());
        log_error("panic", &format!("{} (at {})", message, location));
        previous(info);
    }));
}

/// 读取用户配置。文件不存在（首次运行）、解密失败或 JSON 解析失败时返回 `None`。
pub fn load_config() -> Option<Config> {
    load_config_at(&get_settings_dir().join("config.json"))
}

/// 从指定路径读取配置。
///
/// 抽成带路径的形式，是为了让测试能在临时目录上跑完整的「落盘 → 读回」链路，不必去动
/// 用户真实的 `~/.devops-client`。
fn load_config_at(path: &std::path::Path) -> Option<Config> {
    let encrypted = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            log_error(
                "load_config",
                &format!("read access denied, repairing ACL: {}", e),
            );
            repair_file_acl(path);
            match std::fs::read(path) {
                Ok(b) => b,
                Err(e2) => {
                    log_error(
                        "load_config",
                        &format!("read failed after ACL repair: {}", e2),
                    );
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
    match serde_json::from_slice::<Config>(&data) {
        Ok(config) => Some(config),
        Err(e) => {
            log_error("load_config", &format!("json parse failed: {}", e));
            None
        }
    }
}

/// 保存用户配置到 `~/.devops-client/config.json`（AES-256-GCM 加密）。
pub fn save_config(config: &Config) -> Result<(), String> {
    save_config_at(config, &get_settings_dir().join("config.json"))
}

/// 把配置写到指定路径，供 [`save_config`] 与测试使用。
fn save_config_at(config: &Config, path: &std::path::Path) -> Result<(), String> {
    let dir = path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_else(get_settings_dir);
    std::fs::create_dir_all(&dir).map_err(|e| {
        let m = format!("Failed to create config dir: {}", e);
        log_error("save_config", &m);
        m
    })?;
    restrict_dir_permissions(&dir).inspect_err(|e| log_error("save_config", e))?;

    // Validate language if set
    if !config.language.is_empty() && !SUPPORTED_LANGUAGES.contains(&config.language.as_str()) {
        let m = format!(
            "Unsupported language '{}'. Supported: {:?}",
            config.language, SUPPORTED_LANGUAGES
        );
        log_error("save_config", &m);
        return Err(m);
    }

    // 秘密就在这一份里，由整份文件的 AES-256-GCM 加密保护 —— 不再外移到系统凭据库，
    // 见 README「密码与会话 token 的存放位置」。
    let data = serde_json::to_vec(config).map_err(|e| {
        let m = format!("Failed to serialize config: {}", e);
        log_error("save_config", &m);
        m
    })?;
    let encrypted = crate::crypto::encrypt(&data).inspect_err(|e| log_error("save_config", e))?;
    if let Err(e) = std::fs::write(path, &encrypted) {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            log_error(
                "save_config",
                &format!("write access denied, repairing ACL: {}", e),
            );
            repair_file_acl(path);
            std::fs::write(path, &encrypted).map_err(|e2| {
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
    restrict_file_permissions(path).inspect_err(|e| log_error("save_config", e))?;
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

#[cfg(test)]
mod tests {
    use super::{load_config_at, save_config_at, Config};

    // 升级前的 config.json 带着已删除的证书登记标记：必须仍能解析（多余字段被忽略），
    // 否则老安装升级后连配置都读不出来，用户会被迫重新登录。
    #[test]
    fn legacy_config_with_removed_cert_flag_still_parses() {
        let legacy = r#"{"server_url":"https://example.invalid","token":"t","username":"u","password":"p","cert_registered":true}"#;
        let cfg: Config = serde_json::from_str(legacy).expect("legacy config must still parse");
        assert_eq!(cfg.username, "u");
    }

    /// 旧配置缺少可选字段时必须落到默认值，保证升级路径不会因为字段增减而崩。
    #[test]
    fn legacy_config_without_optional_fields_loads_defaults() {
        let legacy = r#"{"server_url":"https://example.invalid"}"#;
        let cfg: Config = serde_json::from_str(legacy).expect("minimal config must still parse");
        assert!(cfg.token.is_empty());
        assert!(cfg.language.is_empty());
    }

    /// 秘密必须原样落在 `config.json` 里。
    ///
    /// 早前的版本把 token 与密码外移到系统凭据库，文件里只留空串；谁再这么改动一次，
    /// 启动流程就会读不到会话、只能要求用户重新登录，所以把「读写回环后秘密仍在」钉死。
    #[test]
    fn secrets_survive_config_round_trip() {
        let dir =
            std::env::temp_dir().join(format!("devops-client-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("config.json");
        let config = Config {
            server_url: "https://example.invalid".to_string(),
            token: "session-token".to_string(),
            login_at: "2026-01-01 00:00:00".to_string(),
            username: "u".to_string(),
            password: "p".to_string(),
            nickname: "n".to_string(),
            language: "zh".to_string(),
        };

        save_config_at(&config, &path).expect("save must succeed");
        let loaded = load_config_at(&path).expect("load must succeed");
        assert_eq!(loaded.token, "session-token");
        assert_eq!(loaded.password, "p");
        assert_eq!(loaded.username, "u");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
