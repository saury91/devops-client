use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};
use url::Url;

use crate::auth;
use crate::cert;
use crate::config::{load_config, save_config, Config};
use crate::fingerprint;
use crate::i18n::{self, Lang};
use crate::platform;
use crate::state::HeartbeatState;

fn validate_server_url(server_url: &str) -> Result<(), String> {
    if server_url.is_empty() {
        return Err("Server URL is empty".into());
    }
    let parsed = Url::parse(server_url).map_err(|e| format!("Invalid server URL: {}", e))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Server URL must use http:// or https://".into());
    }
    if parsed.host().is_none() {
        return Err("Server URL must include a host".into());
    }
    Ok(())
}

fn get_os_version() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(out) = std::process::Command::new("sw_vers")
            .arg("-productVersion")
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        if let Ok(out) = std::process::Command::new("wmic")
            .args(["os", "get", "Version"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            let s = String::from_utf8_lossy(&out.stdout).to_string();
            if let Some(line) = s.lines().nth(1) {
                let v = line.trim();
                if !v.is_empty() {
                    return v.to_string();
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if let Ok(content) = std::fs::read_to_string("/etc/os-release") {
            for line in content.lines() {
                if let Some(v) = line.strip_prefix("PRETTY_NAME=") {
                    return v.trim_matches('"').to_string();
                }
            }
        }
    }
    std::env::consts::OS.to_string()
}

// --- Platform-specific hardware info collectors ---

#[cfg(target_os = "macos")]
fn collect_macos_info(info: &mut serde_json::Map<String, serde_json::Value>) {
    if let Ok(out) = std::process::Command::new("sh")
        .args([
            "-c",
            "system_profiler SPHardwareDataType 2>/dev/null | awk '/Serial Number/{print $NF}'",
        ])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            info.insert("serial".into(), serde_json::Value::String(s));
        }
    }
    if let Ok(out) = std::process::Command::new("sysctl")
        .args(["-n", "hw.model"])
        .output()
    {
        info.insert(
            "model".into(),
            serde_json::Value::String(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        );
    }
    if let Ok(out) = std::process::Command::new("sysctl")
        .args(["-n", "machdep.cpu.brand_string"])
        .output()
    {
        info.insert(
            "cpu".into(),
            serde_json::Value::String(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        );
    }
    if let Ok(out) = std::process::Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
    {
        let bytes: u64 = String::from_utf8_lossy(&out.stdout)
            .trim()
            .parse()
            .unwrap_or(0);
        info.insert(
            "memory".into(),
            serde_json::Value::String(format!("{} GB", bytes / 1024 / 1024 / 1024)),
        );
    }
    if let Ok(out) = std::process::Command::new("sh")
        .args([
            "-c",
            "df -h / | tail -1 | awk '{print $2\", \"$4\" free\"}'",
        ])
        .output()
    {
        info.insert(
            "disk".into(),
            serde_json::Value::String(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        );
    }
    if let Ok(out) = std::process::Command::new("sh")
        .args([
            "-c",
            "system_profiler SPDisplaysDataType 2>/dev/null | awk '/Chipset Model/{s=$0} /VRAM/{print s\", \"$0; s=\"\"}'",
        ])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            info.insert("gpu".into(), serde_json::Value::String(s));
        }
    }
}

#[cfg(target_os = "windows")]
fn collect_windows_info(info: &mut serde_json::Map<String, serde_json::Value>) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x08000000;

    if let Ok(out) = std::process::Command::new("wmic")
        .args(["bios", "get", "serialnumber"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        if let Some(line) = s.lines().nth(1) {
            let v = line.trim();
            if !v.is_empty() {
                info.insert("serial".into(), serde_json::Value::String(v.to_string()));
            }
        }
    }
    if let Ok(out) = std::process::Command::new("wmic")
        .args(["computersystem", "get", "model"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        let lines: Vec<&str> = s.lines().collect();
        if lines.len() > 1 {
            info.insert(
                "model".into(),
                serde_json::Value::String(lines[1].trim().to_string()),
            );
        }
    }
    if let Ok(out) = std::process::Command::new("wmic")
        .args(["cpu", "get", "name"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        let lines: Vec<&str> = s.lines().collect();
        if lines.len() > 1 {
            info.insert(
                "cpu".into(),
                serde_json::Value::String(lines[1].trim().to_string()),
            );
        }
    }
    if let Ok(out) = std::process::Command::new("wmic")
        .args(["computersystem", "get", "TotalPhysicalMemory"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        if let Some(line) = s.lines().nth(1) {
            if let Ok(bytes) = line.trim().parse::<u64>() {
                info.insert(
                    "memory".into(),
                    serde_json::Value::String(format!("{} GB", bytes / 1024 / 1024 / 1024)),
                );
            }
        }
    }
    if let Ok(out) = std::process::Command::new("wmic")
        .args([
            "logicaldisk",
            "where",
            "DeviceID='C:'",
            "get",
            "Size,FreeSpace",
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        if let Some(line) = s.lines().nth(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                if let (Ok(free), Ok(total)) = (parts[0].parse::<u64>(), parts[1].parse::<u64>()) {
                    info.insert(
                        "disk".into(),
                        serde_json::Value::String(format!(
                            "{} GB, {} GB free",
                            total / 1024 / 1024 / 1024,
                            free / 1024 / 1024 / 1024
                        )),
                    );
                }
            }
        }
    }
    if let Ok(out) = std::process::Command::new("wmic")
        .args(["path", "win32_videocontroller", "get", "name"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).to_string();
        if let Some(line) = s.lines().nth(1) {
            let v = line.trim();
            if !v.is_empty() {
                info.insert("gpu".into(), serde_json::Value::String(v.to_string()));
            }
        }
    }
}

#[cfg(target_os = "linux")]
fn collect_linux_info(info: &mut serde_json::Map<String, serde_json::Value>) {
    if let Ok(out) = std::process::Command::new("sh")
        .args([
            "-c",
            "cat /sys/class/dmi/id/product_serial 2>/dev/null || dmidecode -s system-serial-number 2>/dev/null",
        ])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            info.insert("serial".into(), serde_json::Value::String(s));
        }
    }
    if let Ok(out) = std::process::Command::new("sh")
        .args([
            "-c",
            "cat /sys/class/dmi/id/product_name 2>/dev/null || dmidecode -s system-product-name 2>/dev/null",
        ])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            info.insert("model".into(), serde_json::Value::String(s));
        }
    }
    if let Ok(out) = std::process::Command::new("sh")
        .args([
            "-c",
            "cat /proc/cpuinfo | grep 'model name' | head -1 | cut -d: -f2",
        ])
        .output()
    {
        info.insert(
            "cpu".into(),
            serde_json::Value::String(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        );
    }
    if let Ok(out) = std::process::Command::new("sh")
        .args(["-c", "free -h | grep Mem | awk '{print $2}'"])
        .output()
    {
        info.insert(
            "memory".into(),
            serde_json::Value::String(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        );
    }
    if let Ok(out) = std::process::Command::new("sh")
        .args([
            "-c",
            "df -h / | tail -1 | awk '{print $2\", \"$4\" free\"}'",
        ])
        .output()
    {
        info.insert(
            "disk".into(),
            serde_json::Value::String(String::from_utf8_lossy(&out.stdout).trim().to_string()),
        );
    }
    if let Ok(out) = std::process::Command::new("sh")
        .args([
            "-c",
            "lspci 2>/dev/null | grep -iE 'vga|3d|display' | head -1 | cut -d: -f3-",
        ])
        .output()
    {
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !s.is_empty() {
            info.insert("gpu".into(), serde_json::Value::String(s));
        }
    }
}

fn get_hardware_info_map() -> serde_json::Map<String, serde_json::Value> {
    let mut info = serde_json::Map::new();
    info.insert(
        "hostname".into(),
        serde_json::Value::String(platform::hostname()),
    );
    info.insert(
        "os".into(),
        serde_json::Value::String(platform::os_name().to_string()),
    );
    info.insert(
        "osVersion".into(),
        serde_json::Value::String(get_os_version()),
    );
    info.insert(
        "clientVersion".into(),
        serde_json::Value::String(env!("CARGO_PKG_VERSION").to_string()),
    );

    #[cfg(target_os = "macos")]
    collect_macos_info(&mut info);
    #[cfg(target_os = "windows")]
    collect_windows_info(&mut info);
    #[cfg(target_os = "linux")]
    collect_linux_info(&mut info);

    info
}

fn get_hardware_info() -> String {
    serde_json::Value::Object(get_hardware_info_map()).to_string()
}

// --- i18n helper for auth error classification ---
fn classify_auth_error(lang: Lang, result: &auth::LoginResponse) -> String {
    let code = result.code;
    let msg = &result.msg;
    if code == 401
        || msg.contains("password")
        || msg.contains("credential")
        || msg.contains("密码")
        || msg.contains("用户名")
    {
        return i18n::t(lang, "error.badCredentials").to_string();
    }
    if code == 423 || msg.contains("locked") || msg.contains("锁定") {
        return i18n::t(lang, "error.accountLocked").to_string();
    }
    // Fallback: return the server's own message
    if !msg.is_empty() {
        return msg.clone();
    }
    i18n::t(lang, "login.failed").to_string()
}

// --- Device certificate helpers ---

/// Builds the identity written into the certificate `OU` field.
///
/// User plus host, because neither half alone is enough when an operator inspects the OS key store
/// by hand: a host name cannot tell two accounts on one machine apart, and a user name cannot tell
/// one account's several machines apart.
fn cert_owner(username: &str) -> String {
    let host = platform::hostname();
    if username.is_empty() {
        host
    } else {
        format!("{}@{}", username, host)
    }
}

/// Points Firefox at the key store on the two platforms where it cannot find it by itself.
///
/// Chrome and Safari read the platform key store unprompted, so Firefox is the only browser that
/// needs help: its certificate lives in `CurrentUser\My` on Windows and in the login keychain on
/// macOS, and Firefox presents it once `security.osclientcerts.autoload` is set. Both platforms ship
/// a native `osclientcerts` backend, so this preference is the whole difference between `full` and
/// `partial` there.
///
/// macOS was verified rather than assumed: Firefox 155 presents the login keychain identity at the
/// handshake once the preference is set, and a server demanding a client certificate accepts it.
/// That also means the certificate can stay non-exportable — importing it into a profile's NSS
/// database would require an exportable private key, which would defeat device binding.
///
/// Linux is deliberately excluded: it has no platform key store for Firefox to read, so the
/// preference would be a no-op there and the certificate has to go into each profile's own NSS
/// database instead. That belongs to Linux's own work item; see `cert::linux`.
///
/// Failures are logged instead of returned: the certificate itself is installed and every other
/// browser can use it, so a profile this client cannot write — a snap-confined directory, a locked
/// profile — must not turn a successful install into a failed one.
fn reconcile_browsers() {
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    {
        let update = cert::firefox::enable_os_client_certs();
        if update.failed > 0 {
            crate::config::log_error(
                "reconcile_browsers",
                &format!(
                    "firefox profiles: {}/{} updated, {} unreadable",
                    update.updated, update.profiles, update.failed
                ),
            );
        }
    }
}

/// Installs the certificate into the OS key store and registers it with the server.
///
/// The two halves must happen together: installing without registering leaves the device holding a
/// certificate the server still believes it does not have, and registering without installing
/// leaves the browser with nothing to present at the handshake. Either half alone makes "bound" a
/// lie, so they live in one function.
async fn ensure_cert_bound(
    server_url: &str,
    token: &str,
    username: &str,
) -> Result<cert::CertInfo, String> {
    let status = cert::ensure(&cert_owner(username)).map_err(|e| e.to_string())?;
    let info = status
        .installed
        .ok_or_else(|| "certificate store returned no certificate".to_string())?;
    reconcile_browsers();
    auth::report_cert(server_url, token, false, &info).await?;
    Ok(info)
}

/// Re-issues the certificate inside the renewal window and registers the new fingerprint.
///
/// 顺序是这里唯一重要的事：装新证书（旧的不动）→ 登记到服务端 → 登记成功后才删掉被取代的旧证书。
/// 每一步失败都让设备停在"钥匙串与服务端记录一致"的状态上：
/// - 装不上：旧证书原样可用，什么都没变；
/// - 登记不上：删掉服务端还不知道的新证书（回滚），浏览器出示的仍是服务端记录过的那张；
/// - 删不掉：只多留一张旧证书，而 `status` 取最新 —— 正是刚登记成功的那张。
///
/// @param previous 调用方刚查到的、在服务端记录里的那张证书的指纹
async fn renew_device_cert(
    server_url: &str,
    token: &str,
    owner: &str,
    previous: &str,
) -> Result<(), String> {
    let fresh = cert::install_replacement(owner).map_err(|e| e.to_string())?;

    if let Err(e) = auth::report_cert(server_url, token, true, &fresh).await {
        // 回滚：删掉服务端拒绝登记的新证书，保留仍在服务端记录里的旧证书。少了这一步，
        // 浏览器会出示一张服务端不认识的证书，用户在当前这轮会话里就打不开页面了。
        if let Err(rollback) = cert::keep_only(previous) {
            crate::config::log_error("cert", &format!("renew rollback failed: {}", rollback));
        }
        return Err(e);
    }

    if let Err(e) = cert::keep_only(&fresh.fingerprint) {
        // 新证书已经装好并登记成功；残留的旧证书只是多出一个可选身份，不该让整次续期算失败。
        crate::config::log_error(
            "cert",
            &format!("superseded certificate not removed: {}", e),
        );
    }
    Ok(())
}

/// Re-issues and re-registers the certificate only once it is inside the renewal window.
///
/// @return `Ok(true)` when a renewal actually happened, `Ok(false)` when the certificate is still
///         comfortably valid
async fn renew_if_due(server_url: &str, token: &str, owner: &str) -> Result<bool, String> {
    let Some(info) = cert::status().installed else {
        return Ok(false);
    };
    if !cert::needs_renewal(&info) {
        return Ok(false);
    }
    renew_device_cert(server_url, token, owner, &info.fingerprint).await?;
    Ok(true)
}

// --- Tauri commands ---

#[tauri::command]
pub fn get_lang() -> String {
    match i18n::detect_lang() {
        Lang::En => "en".to_string(),
        Lang::Zh => "zh".to_string(),
    }
}

#[tauri::command]
pub fn get_fingerprint() -> Result<String, String> {
    Ok(fingerprint::get_or_create_fingerprint().value)
}

#[tauri::command]
pub fn load_config_cmd() -> Result<Option<Config>, String> {
    Ok(load_config())
}

#[tauri::command]
pub fn save_config_cmd(config: Config) -> Result<(), String> {
    if !config.server_url.is_empty() {
        if let Err(e) = validate_server_url(&config.server_url) {
            crate::config::log_error("save_config_cmd", &e);
            return Err(e);
        }
    }
    save_config(&config).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_hostname() -> Result<String, String> {
    Ok(platform::hostname())
}

#[tauri::command]
pub fn get_os_info() -> serde_json::Value {
    serde_json::json!({
        "os": platform::os_name(),
        "osVersion": get_os_version(),
        "clientVersion": env!("CARGO_PKG_VERSION")
    })
}

#[tauri::command]
pub async fn do_login(
    server_url: String,
    username: String,
    password: String,
    device_name: String,
) -> Result<serde_json::Value, String> {
    validate_server_url(&server_url)?;
    let fp = fingerprint::get_or_create_fingerprint();
    let lang = i18n::detect_lang();
    let os = platform::os_name();
    let device_info = get_hardware_info();

    let result = auth::login_device(
        &server_url,
        &username,
        &password,
        &fp.value,
        &device_name,
        os,
        &get_os_version(),
        env!("CARGO_PKG_VERSION"),
        &device_info,
    )
    .await;

    match result {
        Err(auth::LoginError::Network(detail)) => {
            Err(format!("{}: {}", i18n::t(lang, "login.connFailed"), detail))
        }
        Err(auth::LoginError::Server(_code, msg)) => Err(if !msg.is_empty() {
            msg
        } else {
            i18n::t(lang, "login.failed").to_string()
        }),
        Ok(resp) if resp.code != 200 => Err(classify_auth_error(lang, &resp)),
        Ok(resp) => {
            let data = resp
                .data
                .ok_or_else(|| i18n::t(lang, "error.serverError").to_string())?;
            let status = data.status.unwrap_or_else(|| "error".to_string());
            let token = data.token.unwrap_or_default();

            // `login-device` answers `ok` for a device that is approved and holds a session, and
            // `pending` — with no token — while it waits for approval. That is a different
            // vocabulary from `device-status`, which speaks `active` / `pending` / `revoked`:
            // testing for `active` here never matched, so the certificate was silently never
            // registered. Binding stays gated on the token because the server refuses `bind-cert`
            // for a device that is not approved yet, and an early attempt would surface as a
            // spurious "waiting for approval" error.
            let (cert_fingerprint, cert_warning) = if status == "ok" && !token.is_empty() {
                match ensure_cert_bound(&server_url, &token, &username).await {
                    Ok(info) => (Some(info.fingerprint), None),
                    // A certificate failure must not block the sign-in: the whole point of grading
                    // device capability is that a machine which cannot hold a certificate still
                    // gets in. The reason goes back to the UI instead.
                    Err(e) => {
                        crate::config::log_error(
                            "do_login",
                            &format!("install cert failed: {}", e),
                        );
                        (None, Some(e))
                    }
                }
            } else {
                (None, None)
            };

            let cert_capability = cert::reported_capability();
            // A machine that cannot host a certificate at all (`unavailable`) counts as settled
            // even with nothing registered: another sign-in cannot give it one, so leaving it
            // unsettled would make it demand a login on every start. Everywhere else an
            // unregistered certificate stays unsettled on purpose — the UI answers that with one
            // more sign-in, which is where registration happens, and the flag is also what tells
            // an upgraded install (whose sign-ins never registered anything) from a settled one.
            let cert_registered =
                cert_fingerprint.is_some() || cert_capability == cert::CertCapability::Unavailable;

            Ok(serde_json::json!({
                "status": status,
                "token": token,
                "message": data.message.unwrap_or_default(),
                "fingerprint": fp.value,
                "certFingerprint": cert_fingerprint,
                // What the certificate can actually do, not the platform ceiling: a machine whose
                // certificate ended up untrusted — a refused grant, a key store that rejected the
                // write — must reach the UI as `partial` so it can be reported, instead of looking
                // like every working machine.
                "certCapability": cert_capability.as_str(),
                "certRegistered": cert_registered,
                "certWarning": cert_warning
            }))
        }
    }
}

#[tauri::command]
pub async fn get_user_info(server_url: String, token: String) -> Result<serde_json::Value, String> {
    validate_server_url(&server_url)?;
    let lang = i18n::detect_lang();
    let resp = auth::get_user_info(&server_url, &token).await?;
    if resp.code != 200 {
        return Err(resp.msg);
    }
    let info = resp
        .data
        .ok_or_else(|| i18n::t(lang, "error.serverError").to_string())?;
    Ok(serde_json::json!({
        "id": info.id,
        "username": info.username.unwrap_or_default(),
        "nickname": info.nickname.unwrap_or_default(),
        "avatar": info.avatar.unwrap_or_default()
    }))
}

#[tauri::command]
pub async fn change_password(
    server_url: String,
    token: String,
    old_password: String,
    new_password: String,
) -> Result<(), String> {
    validate_server_url(&server_url)?;
    auth::change_password(&server_url, &token, &old_password, &new_password).await
}

#[tauri::command]
pub async fn auto_login(
    server_url: String,
    fingerprint: String,
) -> Result<serde_json::Value, String> {
    validate_server_url(&server_url)?;
    let lang = i18n::detect_lang();
    let resp = auth::auto_login(&server_url, &fingerprint).await?;
    if resp.code != 200 {
        return Err(resp.msg);
    }
    let data = resp
        .data
        .ok_or_else(|| i18n::t(lang, "error.serverError").to_string())?;
    let token = data.token.unwrap_or_default();
    Ok(serde_json::json!({ "token": token }))
}

/// 向服务端上报本机已离线（`POST /api/auth/device-offline`），失败只记日志。
///
/// 服务端据此立刻清掉本设备的在场标记并失效这台设备的会话，不必等标记 TTL 自然过期；
/// 请求没发出去时语义仍然正确，只是退化成超时兜底。
///
/// @param server_url 服务端地址
/// @param token      当前会话 ID
/// @param timeout    请求超时：登出路径可以等，退出路径必须短
async fn post_device_offline(server_url: &str, token: &str, timeout: Duration) {
    let client = match reqwest::Client::builder()
        .danger_accept_invalid_certs(false)
        .timeout(timeout)
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            crate::config::log_error("logout", &format!("failed to build http client: {}", e));
            return;
        }
    };

    // Only call device-offline — /logout is a browser-side convenience path
    let url = format!(
        "{}/api/auth/device-offline",
        server_url.trim_end_matches('/')
    );
    // 只记日志，不往上返回错误：调用方都已经完成了本地动作（清 token / 退出进程），这一步只是
    // 尽量让服务端早点把这台设备标记为离线，服务端不可达不该让用户卡在流程里。
    if let Err(e) = client.post(&url).header("X-Session-Id", token).send().await {
        crate::config::log_error("logout", &format!("device-offline request failed: {}", e));
    }
}

#[tauri::command]
pub async fn server_logout(server_url: String, token: String) -> Result<(), String> {
    if !server_url.is_empty() {
        validate_server_url(&server_url)?;
    }
    post_device_offline(&server_url, &token, Duration::from_secs(10)).await;
    Ok(())
}

/// 退出进程前尽力通知服务端本机已离线。
///
/// 由 `RunEvent::ExitRequested` 调用，覆盖托盘「退出」、面板的退出按钮、macOS 的 Cmd+Q 等全部出口：
/// 服务端会立刻失效这台设备的桌面端会话，以及由客户端打开的浏览器工作台会话，而不是等在场标记的
/// TTL（默认 300 秒）自然过期。
///
/// 超时取 2 秒（登出路径是 10 秒）：退出时用户已经在等进程结束，服务端不可达不该让窗口僵住。
/// 被强杀（SIGKILL）时本函数根本不会执行，那种情况仍由心跳中断兜底。
pub fn notify_device_offline() {
    let config = match load_config() {
        Some(c) => c,
        None => return,
    };
    if config.server_url.is_empty() || config.token.is_empty() {
        return;
    }
    tauri::async_runtime::block_on(post_device_offline(
        &config.server_url,
        &config.token,
        Duration::from_secs(2),
    ));
}

/// Reports what this machine can do and whether a certificate is installed, without installing one.
#[tauri::command]
pub fn get_cert_status() -> cert::CertStatus {
    cert::status()
}

/// Installs the device certificate and, when a session is available, registers it with the server.
///
/// An empty `server_url` or `token` installs into the key store only, which is what the UI's
/// "retry install" button needs before a session exists. When both are present the fingerprint is
/// reported right away, so the intermediate state of "installed locally, unknown to the server"
/// never outlives this call.
///
/// @param server_url server address, may be empty
/// @param token      session id, may be empty
/// @param username   signed-in user, written into the `OU` field
/// @return the resulting status, or a description when the key store refused
#[tauri::command]
pub async fn install_device_cert(
    server_url: String,
    token: String,
    username: String,
) -> Result<cert::CertStatus, String> {
    if !server_url.is_empty() {
        validate_server_url(&server_url)?;
    }

    let status = cert::ensure(&cert_owner(&username)).map_err(|e| e.to_string())?;

    // Retrying an install is the one path that can repair a profile Firefox could not read earlier,
    // so browser reconciliation happens here too and not only during sign-in.
    reconcile_browsers();

    if !server_url.is_empty() && !token.is_empty() {
        if let Some(info) = status.installed.as_ref() {
            auth::report_cert(&server_url, &token, false, info).await?;
        }
    }

    Ok(status)
}

#[tauri::command]
pub fn open_browser(url: String) -> Result<(), String> {
    let parsed = Url::parse(&url).map_err(|e| format!("Invalid URL: {}", e))?;
    if parsed.scheme() != "http" && parsed.scheme() != "https" {
        return Err("Only http/https URLs are allowed".into());
    }

    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg(&url).spawn();

    #[cfg(target_os = "windows")]
    let result = {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        // 用 rundll32 直接调默认浏览器，不经 cmd shell，& 不会被当作命令分隔符，查询参数完整保留
        std::process::Command::new("rundll32")
            .arg("url.dll,FileProtocolHandler")
            .arg(&url)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
    };

    #[cfg(target_os = "linux")]
    let result = std::process::Command::new("xdg-open").arg(&url).spawn();

    #[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
    let result = std::process::Command::new("xdg-open").arg(&url).spawn();

    result.map_err(|e| format!("Failed to open browser: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn open_dashboard(server_url: String, token: String) -> Result<(), String> {
    let url = build_dashboard_url(&server_url, &token).await?;
    open_browser(url)
}

#[tauri::command]
pub async fn get_dashboard_url(server_url: String, token: String) -> Result<String, String> {
    build_dashboard_url(&server_url, &token).await
}

/// 构建打开工作台的 exchange URL（创建 exchange token 但不打开浏览器），
/// 供“长按复制链接”使用，与 open_dashboard 走同一套逻辑。
async fn build_dashboard_url(server_url: &str, token: &str) -> Result<String, String> {
    if token.is_empty() {
        return Ok(server_url.trim_end_matches('/').to_string());
    }

    // The browser session this URL opens is about to be authenticated with the certificate, so
    // settle its trust first: a terminal upgraded from a build that never granted trust holds one
    // no browser will present, and finding that out at the handshake costs the user a session that
    // simply fails. This never installs, so it cannot invent an identity the server has not seen,
    // and a failure is logged rather than returned — the certificate is one of several checks the
    // server applies, and losing the URL over it would be worse than opening a session that may
    // fall back.
    if let Err(e) = cert::repair_installed() {
        crate::config::log_error(
            "open_dashboard",
            &format!("cert trust repair failed: {}", e),
        );
    }

    let exchange_token = auth::create_exchange_token(server_url, token).await?;
    let base = server_url.trim_end_matches('/');
    let mut url = Url::parse(base).map_err(|e| e.to_string())?;
    url.set_path("/api/auth/exchange-token");
    url.query_pairs_mut()
        .append_pair("exchangeToken", &exchange_token);
    Ok(url.to_string())
}

/// 心跳间隔：调大可降低服务端压力（客户端越多越需要），但设备撤销/会话失效检测会变慢
const HEARTBEAT_INTERVAL_SECS: u64 = 30;

/// 无论心跳线程怎么退出（正常结束、break、panic 展开），都把共享标志复位。
///
/// 没有它的话，循环里任何一次 panic 都会带着 `running = true` 解栈：此后每次 `start_heartbeat`
/// 都在这个标志上提前返回，心跳在本进程剩余生命周期里再也不会恢复 —— 用户看到的是一个"已登录、
/// 但服务端再也收不到状态"的客户端，只能重启应用。
struct HeartbeatReset {
    state: Arc<HeartbeatState>,
}

impl Drop for HeartbeatReset {
    fn drop(&mut self) {
        self.state.running.store(false, Ordering::SeqCst);
        // `lock()` 报 Err 只在持锁线程 panic 过时才出现，而这条路径本身常常就在 panic 展开中：
        // 再 panic 一次会变成 abort，所以中毒的锁按"已清空"处理，不 unwrap。
        if let Ok(mut cancel) = self.state.cancel.lock() {
            *cancel = None;
        }
    }
}

/// 会话在服务端已失效时，按指纹免密重建一个。
///
/// 心跳是设备在场的唯一依据：循环一旦退出，在场标记到期后这台设备就会被判为离线，用户打开
/// 工作台只能看到离线页，且必须重新登录才能恢复。触发条件很常见——笔记本合盖睡眠期间进程被
/// 挂起，会话在 Redis 里到期，唤醒后第一次心跳就会拿到 `SESSION_INVALID`。因此这里不能退出
/// 循环，而要走启动时「静默续登」的同一条路径把会话换回来。
///
/// 设备被撤销、未登记，或服务端不可达时返回 `Err`，由调用方退回原行为（回登录页）：那时用户
/// 需要看到原因并自己处理，而不是留一个「界面说已登录、服务端却收不到心跳」的客户端。
///
/// 新会话必须落盘。心跳每轮都从配置文件重读 token，写不进去就会拿旧会话反复触发本函数，
/// 于是每 30 秒白跑一次免密登录；而工作台兑换令牌、退出时的离线上报读的同样是这份配置。
///
/// @param server_url  服务端地址
/// @param fingerprint 本机设备指纹
/// @return 重建后的会话 ID
async fn renew_session(server_url: &str, fingerprint: &str) -> Result<String, String> {
    let resp = crate::auth::auto_login(server_url, fingerprint).await?;
    if resp.code != 200 {
        return Err(resp.msg);
    }
    let token = resp
        .data
        .and_then(|d| d.token)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| "auto-login returned an empty token".to_string())?;

    let mut config = load_config().ok_or_else(|| "config is missing".to_string())?;
    config.token = token.clone();
    save_config(&config)?;
    Ok(token)
}

#[tauri::command]
pub fn start_heartbeat(
    server_url: String,
    fingerprint: String,
    heartbeat_state: State<'_, Arc<HeartbeatState>>,
    app_handle: AppHandle,
) -> Result<(), String> {
    validate_server_url(&server_url)?;

    // Serialize all start attempts
    let _lock = heartbeat_state.start_lock.lock().unwrap();

    if heartbeat_state.running.load(Ordering::SeqCst) {
        return Ok(());
    }
    heartbeat_state.running.store(true, Ordering::SeqCst);

    let cancel = Arc::new(AtomicBool::new(true));
    *heartbeat_state.cancel.lock().unwrap() = Some(cancel.clone());

    let heartbeat_state = heartbeat_state.inner().clone();

    std::thread::spawn(move || {
        // 放在线程最外层：创建运行时的失败、循环里的 panic 都经由它的 Drop 复位标志。
        let _reset = HeartbeatReset {
            state: heartbeat_state.clone(),
        };
        let rt = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            // 拿不到运行时就连循环都进不去，直接退出让标志复位，下次 start_heartbeat 可以重试。
            Err(e) => {
                crate::config::log_error("heartbeat", &format!("runtime init failed: {}", e));
                return;
            }
        };
        rt.block_on(async move {
            let mut failures: u32 = 0;
            let mut last_failure_time: Option<std::time::Instant> = None;
            // Renewal is measured in days, so there is no reason to probe the key store on every
            // heartbeat cycle; this counter limits the check to roughly once per ten minutes.
            let mut cycle: u64 = 0;
            let mut renew_retry_after: Option<std::time::Instant> = None;
            loop {
                if !cancel.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS)).await;
                if !cancel.load(Ordering::SeqCst) {
                    break;
                }

                // Re-read token each cycle so session renewals are picked up
                let config = load_config();
                let token = config.as_ref().map(|c| c.token.clone()).unwrap_or_default();
                if token.is_empty() {
                    let _ = app_handle.emit("connection-lost", ());
                    break;
                }
                let owner = cert_owner(
                    config
                        .as_ref()
                        .map(|c| c.username.as_str())
                        .unwrap_or_default(),
                );

                cycle = cycle.wrapping_add(1);

                // Roughly every ten minutes: settle the trust of the certificate already installed.
                // Placed before the capability below so a repair is reported in this same cycle
                // rather than the next, and deliberately not conditional on the status the request
                // below returns — an unusable certificate is a local fault, and the server is the
                // party that has to hear about it.
                //
                // The machine this exists for was registered by a build that never granted trust,
                // and neither a silent sign-in nor this loop passes through `ensure`: without this
                // it would stay broken for as long as it keeps its session and never types a
                // password again, while every browser session it opened was rejected.
                if cycle % 20 == 1 {
                    if let Err(e) = cert::repair_installed() {
                        crate::config::log_error(
                            "heartbeat",
                            &format!("cert trust repair failed: {}", e),
                        );
                    }
                }

                // Report what this machine can actually do, not its platform ceiling. The value is
                // stored against the device, and a certificate no browser will present must not be
                // reported as `full` — the value a working machine reports, which is what leaves
                // the two indistinguishable. The ceiling still answers while nothing is installed,
                // so a device that cannot host a certificate keeps reporting `unavailable` and is
                // never mistaken for one whose certificate went missing. Recomputed every cycle
                // rather than cached, so a change such as a Linux user installing the NSS tools is
                // picked up without a restart.
                match auth::check_device_status(
                    &server_url,
                    &fingerprint,
                    &token,
                    cert::reported_capability().as_str(),
                )
                .await
                {
                    Ok(crate::auth::DeviceStatus::Revoked) => {
                        let _ = app_handle.emit("device-revoked", ());
                        app_handle.exit(0);
                        break;
                    }
                    Ok(crate::auth::DeviceStatus::Active)
                    | Ok(crate::auth::DeviceStatus::Pending) => {
                        failures = 0;
                        let _ = app_handle.emit("heartbeat-ok", ());

                        // Roughly every ten minutes, and never while a previous attempt is still
                        // in its backoff window.
                        if cycle % 20 == 1
                            && renew_retry_after.is_none_or(|t| t <= std::time::Instant::now())
                        {
                            match renew_if_due(&server_url, &token, &owner).await {
                                Ok(true) => {
                                    let _ = app_handle.emit("cert-renewed", ());
                                }
                                Ok(false) => {}
                                Err(e) => {
                                    crate::config::log_error(
                                        "heartbeat",
                                        &format!("cert renew failed: {}", e),
                                    );
                                    // Back off for ten minutes: the failed attempt rolled back, so
                                    // a retry is safe, but each attempt mints a new key pair and
                                    // leaves two certificates behind until a registration succeeds.
                                    // Once every ten minutes recovers on its own without churning
                                    // the key store on every heartbeat.
                                    renew_retry_after =
                                        Some(std::time::Instant::now() + Duration::from_secs(600));
                                }
                            }
                        }
                    }
                    Ok(crate::auth::DeviceStatus::Error(ref reason))
                        if reason == "SESSION_INVALID" =>
                    {
                        // 会话在服务端过期（合盖睡眠、长时间无请求等）。这里自愈而不是退出循环，
                        // 否则在场标记随之停更，用户会被判为离线并被迫重新登录，见 renew_session。
                        match renew_session(&server_url, &fingerprint).await {
                            Ok(token) => {
                                crate::config::log_error(
                                    "heartbeat",
                                    "session expired; re-established silently by fingerprint",
                                );
                                // 界面持有的是登录那一刻的 token，不通知就会用旧会话去兑换工作台令牌
                                let _ = app_handle.emit("session-renewed", token);
                                failures = 0;
                                let _ = app_handle.emit("heartbeat-ok", ());
                                continue;
                            }
                            Err(e) => {
                                crate::config::log_error(
                                    "heartbeat",
                                    &format!("session expired and silent re-login failed: {}", e),
                                );
                                let _ = app_handle.emit("connection-lost", ());
                                break;
                            }
                        }
                    }
                    Ok(crate::auth::DeviceStatus::Error(ref reason))
                        if reason == "FINGERPRINT_MISMATCH" =>
                    {
                        // 指纹对不上说明这个会话不属于本机：服务端已把它连同请求指纹归属账号的
                        // 会话一并失效。这是安全事件而不是网络故障，既不做免密重建，也不重试
                        // 心跳；单独发事件让界面提示「重新登录」，并本地留痕便于事后审计。
                        crate::config::log_error(
                            "heartbeat",
                            "fingerprint does not match this session; heartbeat stopped, \
                             identity mismatch reported",
                        );
                        let _ = app_handle.emit("device-identity-mismatch", ());
                        break;
                    }
                    Ok(crate::auth::DeviceStatus::Error(_))
                    | Ok(crate::auth::DeviceStatus::NotFound) => {
                        failures += 1;
                        last_failure_time = Some(std::time::Instant::now());
                        let _ = app_handle.emit("heartbeat-fail", ());
                        if failures >= 3 {
                            let _ = app_handle.emit("connection-lost", ());
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    Err(_) => {
                        failures += 1;
                        last_failure_time = Some(std::time::Instant::now());
                        let _ = app_handle.emit("heartbeat-fail", ());
                        if failures >= 3 {
                            let _ = app_handle.emit("connection-lost", ());
                            break;
                        }
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }

                // Decay failure count if more than 5 minutes since last failure
                if let Some(ref last) = last_failure_time {
                    if last.elapsed() > Duration::from_secs(300) && failures > 0 {
                        failures = failures.saturating_sub(1);
                        last_failure_time = Some(std::time::Instant::now());
                    }
                }
            }
        });
    });

    Ok(())
}

#[tauri::command]
pub fn stop_heartbeat(heartbeat_state: State<'_, Arc<HeartbeatState>>) -> Result<(), String> {
    heartbeat_state.running.store(false, Ordering::SeqCst);
    if let Some(cancel) = heartbeat_state.cancel.lock().unwrap().take() {
        cancel.store(false, Ordering::SeqCst);
    }
    Ok(())
}

/// 当前是否为本地调试构建（`tauri dev` / `cargo run`）。
///
/// 调试窗口和正式安装包在界面上长得一模一样，用户无法分辨眼前这个是开发时随手起的
/// 还是装到机器上的那一份。前端据此在标题栏打上 DEV 标识。
#[tauri::command]
pub fn is_dev_build() -> bool {
    cfg!(debug_assertions)
}

/// 首屏渲染完成后显示主窗口。
///
/// 主窗口以 `visible: false` 创建：webview 从加载 HTML 到画出第一帧这段时间里，窗口只是
/// 一块 `backgroundColor` 纯色，用户看到的就是“点开图标弹出一个空白页”。等前端把视图、
/// 语言、尺寸都准备好再调这里，第一帧就是最终界面。
#[tauri::command]
pub fn show_main_window(app_handle: AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        window.show().map_err(|e| e.to_string())?;
        let _ = window.set_focus();
    }
    Ok(())
}

#[tauri::command]
pub fn resize_window(app_handle: AppHandle, width: f64, height: f64) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        window
            .set_size(tauri::LogicalSize::new(width, height))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn minimize_window(app_handle: AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        window.minimize().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn quit_app(
    app_handle: AppHandle,
    heartbeat_state: State<'_, Arc<HeartbeatState>>,
) -> Result<(), String> {
    heartbeat_state.running.store(false, Ordering::SeqCst);
    if let Some(cancel) = heartbeat_state.cancel.lock().unwrap().take() {
        cancel.store(false, Ordering::SeqCst);
    }
    app_handle.exit(0);
    Ok(())
}

#[tauri::command]
pub fn hide_window(app_handle: AppHandle) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        window.hide().map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub fn start_drag(window: tauri::WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_device_info() -> serde_json::Value {
    serde_json::Value::Object(get_hardware_info_map())
}

#[tauri::command]
pub async fn test_connection(url: String) -> Result<serde_json::Value, String> {
    let start = std::time::Instant::now();
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(false)
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .map_err(|e| e.to_string())?;
    let ping_url = format!("{}/api/auth/ping", url.trim_end_matches('/'));
    let resp = client
        .get(&ping_url)
        .send()
        .await
        .map_err(|e| format!("connect failed: {}", e))?;
    let status = resp.status().as_u16();
    // 非 JSON 响应不能静默换成 null：反代/门户挂在同一个地址上时会返回一页 HTML 且状态码仍是 200，
    // `unwrap_or_default()` 会把这种情况报成"连接成功"，用户拿着一个不是本服务端的地址也能通过测试。
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("unexpected response body (status {}): {}", status, e))?;
    let latency = start.elapsed().as_millis() as u64;
    Ok(serde_json::json!({
        "ok": status == 200,
        "status": status,
        "latency": latency,
        "body": body
    }))
}

#[tauri::command]
pub fn export_log_file(content: String, path: String) -> Result<(), String> {
    if let Some(parent) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(&path, &content).map_err(|e| e.to_string())?;
    Ok(())
}

/// 读取后端错误日志（含 panic 记录）的尾部，供「导出日志」附加在用户事件日志之后。
///
/// 只取尾部：文件本身有 1 MiB 上限（超过会轮转），而一次故障里真正有用的永远是最后一段；
/// 无上限地整份读取，会让附件在不该大的时候变大。
#[tauri::command]
pub fn read_error_log(max_bytes: usize) -> Result<String, String> {
    let path = crate::config::get_settings_dir().join("error.log");
    if !path.exists() {
        // 全新安装、或运行至今没出过错。返回空串而不是错误，调用方按「没有后端日志」处理。
        return Ok(String::new());
    }
    let data = std::fs::read(&path).map_err(|e| format!("Failed to read error log: {}", e))?;
    let start = data.len().saturating_sub(max_bytes);
    // 从任意字节偏移切分可能落在多字节字符中间，lossy 转换会把那个残字换成替换符 ——
    // 比返回 Err 把整段日志丢掉好。
    Ok(String::from_utf8_lossy(&data[start..]).into_owned())
}

#[tauri::command]
pub fn export_device_key() -> Result<String, String> {
    use base64ct::{Base64, Encoding};
    let dir = crate::config::get_settings_dir();
    let path = dir.join("device.key");
    let data = std::fs::read(&path).map_err(|e| format!("Failed to read device key: {}", e))?;
    Ok(Base64::encode_string(&data))
}

#[tauri::command]
pub fn import_device_key(b64: String) -> Result<(), String> {
    use base64ct::{Base64, Encoding};
    let data = Base64::decode_vec(&b64).map_err(|e| format!("Invalid base64: {}", e))?;
    // 先验证再落盘：导入的内容会直接成为这台设备的身份，写进去一份读不出来的数据会让之后每个
    // 请求都签不出来，用户只能重新登录才能恢复 —— 备份文件在那里，但不该指望人去手工还原。
    if !crate::fingerprint::is_restorable_device_key(&data) {
        return Err("Not a usable device key".to_string());
    }
    let dir = crate::config::get_settings_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    let path = dir.join("device.key");
    // Backup existing key before overwriting
    if path.exists() {
        let backup = path.with_extension("key.bak");
        std::fs::copy(&path, &backup).map_err(|e| format!("Failed to backup: {}", e))?;
    }
    // 写的是与 fingerprint.rs 同一个文件，权限也必须一样（0600）：原来直接 fs::write，
    // 导入过的 device.key 是 0644 —— 本机其他账号能读走设备私钥。
    crate::fingerprint::write_key_file(&path, &data)
        .map_err(|e| format!("Failed to write: {}", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{HeartbeatReset, HeartbeatState};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    /// 心跳线程在跑的状态：`running` 为真、有一个取消标志。
    fn running_state() -> Arc<HeartbeatState> {
        Arc::new(HeartbeatState {
            running: AtomicBool::new(true),
            cancel: Mutex::new(Some(Arc::new(AtomicBool::new(false)))),
            start_lock: Mutex::new(()),
        })
    }

    /// 回归点：心跳线程 panic 时 `running` 必须被复位。否则解栈后 `running` 恒为真，
    /// 之后每次 `start_heartbeat` 都提前返回，心跳到进程结束都不会再恢复。
    #[test]
    fn heartbeat_flags_are_cleared_when_the_worker_goes_away() {
        let state = running_state();
        drop(HeartbeatReset {
            state: state.clone(),
        });

        assert!(!state.running.load(Ordering::SeqCst));
        assert!(state.cancel.lock().unwrap().is_none());
    }

    /// 这个 Drop 有可能正是在别的线程 panic 展开的路上跑的；此时锁已经中毒，
    /// 若这里再 unwrap 一次 panic，整个进程会 abort —— 比"心跳停了"严重得多。
    /// 下面的 poison 线程会打印一条 panic 信息，是测试预期内的输出。
    #[test]
    fn heartbeat_reset_survives_a_poisoned_lock() {
        let state = running_state();
        let poisoner = state.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.cancel.lock().unwrap();
            panic!("poison the mutex on purpose");
        })
        .join();
        assert!(state.cancel.is_poisoned());

        drop(HeartbeatReset {
            state: state.clone(),
        });

        assert!(!state.running.load(Ordering::SeqCst));
    }
}
