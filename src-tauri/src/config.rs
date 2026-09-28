use crate::secret;
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
    /// 本安装是否已完成过一次「登录时登记设备证书」的交互式登录。
    ///
    /// 旧版本客户端的登录从不登记证书，升级上来的配置里没有这个字段（`serde(default)` 得到
    /// `false`）。前端启动流程据此识别这类安装：不静默续登，而是清掉会话回到登录页重新登录
    /// 一次，由交互式登录路径补做证书登记。彻底装不了证书的机器（`unavailable`）由能力口径
    /// 视为已登记，否则它每次启动都会被要求重新登录。
    #[serde(default)]
    pub cert_registered: bool,
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
/// 之外的 panic 既不会结束进程、也不会打印到任何地方：心跳线程、代理线程、证书路径上
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

/// 把凭据库里的秘密填回内存中的配置。
///
/// 文件里的对应字段为空 = 秘密在凭据库里。字段非空则是升级上来的老配置（秘密还在文件
/// 里），下一次 `save_config` 会把它迁走，这里不动它。
fn hydrate_secrets(config: &mut Config) {
    if !secret::is_supported() {
        return;
    }
    if config.token.is_empty() {
        if let Some(value) = secret::load(secret::ACCOUNT_TOKEN) {
            config.token = value;
        }
    }
    if config.password.is_empty() {
        if let Some(value) = secret::load(secret::ACCOUNT_PASSWORD) {
            config.password = value;
        }
    }
}

/// 把一个秘密交给凭据库，并从即将落盘的那份配置里抹掉。
///
/// 返回 `true` 表示调用方应当清空文件里的值。凭据库不可用（Linux）或写入失败时返回
/// `false`，字段保持原样写进加密的 `config.json` —— 宁可降级，也不能让用户重新输密码。
///
/// 空值按「用户主动清空」处理（登出流程会走到这里），此时要删掉凭据库里的条目，否则下
/// 次加载又会把旧秘密填回来，等于「退出登录没清掉密码」。
fn offload_secret(account: &str, value: &str) -> bool {
    if !secret::is_supported() {
        return false;
    }
    if value.is_empty() {
        let _ = secret::delete(account);
        return true;
    }
    match secret::store(account, value) {
        Ok(()) => true,
        Err(e) => {
            log_error(
                "save_config",
                &format!(
                    "cannot store '{}' in credential store, keeping it in config.json: {}",
                    account, e
                ),
            );
            false
        }
    }
}

pub fn load_config() -> Option<Config> {
    let path = get_settings_dir().join("config.json");
    let encrypted = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            log_error(
                "load_config",
                &format!("read access denied, repairing ACL: {}", e),
            );
            repair_file_acl(&path);
            match std::fs::read(&path) {
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
        Ok(mut config) => {
            // 密码与 token 落在系统凭据库里，文件里对应字段是空的，读出来要补回去。
            hydrate_secrets(&mut config);
            Some(config)
        }
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

    // 落盘的那一份不能带秘密：config.json 的密钥是可推导的（见 secret.rs 顶部的说明），
    // 所以先把密码与 token 交给系统凭据库，再序列化剩下的一份。
    let mut stored = config.clone();
    if offload_secret(secret::ACCOUNT_TOKEN, &config.token) {
        stored.token.clear();
    }
    if offload_secret(secret::ACCOUNT_PASSWORD, &config.password) {
        stored.password.clear();
    }

    let data = serde_json::to_vec(&stored).map_err(|e| {
        let m = format!("Failed to serialize config: {}", e);
        log_error("save_config", &m);
        m
    })?;
    let encrypted = crate::crypto::encrypt(&data).inspect_err(|e| log_error("save_config", e))?;
    let path = dir.join("config.json");
    if let Err(e) = std::fs::write(&path, &encrypted) {
        if e.kind() == std::io::ErrorKind::PermissionDenied {
            log_error(
                "save_config",
                &format!("write access denied, repairing ACL: {}", e),
            );
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
    restrict_file_permissions(&path).inspect_err(|e| log_error("save_config", e))?;
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
    use super::Config;

    // 升级前的 config.json 没有 cert_registered 字段：必须仍能解析，并落到「未登记」，
    // 启动流程据此把老安装退回登录页重新登录一次，由交互式登录补齐证书登记。
    #[test]
    fn legacy_config_without_cert_registered_loads_as_unregistered() {
        let legacy =
            r#"{"server_url":"https://example.invalid","token":"t","username":"u","password":"p"}"#;
        let cfg: Config = serde_json::from_str(legacy).expect("legacy config must still parse");
        assert!(!cfg.cert_registered);
    }
}
