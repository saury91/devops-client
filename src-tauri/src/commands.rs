use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};
use url::Url;

use crate::auth;
use crate::config::{load_config, save_config, Config};
use crate::fingerprint;
use crate::i18n::{self, Lang};
use crate::platform;
use crate::proxy;
use crate::state::{HeartbeatState, ProxyState};

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
                if let (Ok(free), Ok(total)) = (parts[0].parse::<u64>(), parts[1].parse::<u64>())
                {
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
        .args(["-c", "df -h / | tail -1 | awk '{print $2\", \"$4\" free\"}'"])
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
    if code == 401 || msg.contains("password") || msg.contains("credential") || msg.contains("密码") || msg.contains("用户名") {
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
        validate_server_url(&config.server_url)?;
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
        Err(auth::LoginError::Server(_code, msg)) => {
            Err(if !msg.is_empty() { msg } else { i18n::t(lang, "login.failed").to_string() })
        }
        Ok(resp) if resp.code != 200 => {
            Err(classify_auth_error(lang, &resp))
        }
        Ok(resp) => {
            let data = resp
                .data
                .ok_or_else(|| i18n::t(lang, "error.serverError").to_string())?;
            let status = data.status.unwrap_or_else(|| "error".to_string());
            let token = data.token.unwrap_or_default();

            Ok(serde_json::json!({
                "status": status,
                "token": token,
                "message": data.message.unwrap_or_default(),
                "fingerprint": fp.value
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
    let info = resp.data.ok_or_else(|| i18n::t(lang, "error.serverError").to_string())?;
    Ok(serde_json::json!({
        "id": info.id,
        "username": info.username.unwrap_or_default(),
        "nickname": info.nickname.unwrap_or_default(),
        "avatar": info.avatar.unwrap_or_default()
    }))
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
    let data = resp.data.ok_or_else(|| i18n::t(lang, "error.serverError").to_string())?;
    let token = data.token.unwrap_or_default();
    Ok(serde_json::json!({ "token": token }))
}

#[tauri::command]
pub async fn server_logout(server_url: String, token: String) -> Result<(), String> {
    if !server_url.is_empty() {
        validate_server_url(&server_url)?;
    }
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(false)
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;

    let base = server_url.trim_end_matches('/');
    // Only call device-offline — /logout is a browser-side convenience path
    let url = format!("{}/api/auth/device-offline", base);
    let _ = client
        .post(&url)
        .header("X-Session-Id", &token)
        .send()
        .await;

    Ok(())
}

#[tauri::command]
pub fn start_proxy(
    fingerprint: String,
    proxy_state: State<'_, Arc<ProxyState>>,
    app_handle: AppHandle,
) -> Result<u16, String> {
    // Serialize all start attempts to prevent double-start
    let _lock = proxy_state.start_lock.lock().unwrap();

    if proxy_state.running.load(Ordering::SeqCst) {
        if let Some(port) = *proxy_state.port.lock().unwrap() {
            return Ok(port);
        }
    }

    let (port, shutdown_tx) = proxy::start_proxy(fingerprint.clone(), app_handle)?;
    proxy_state.running.store(true, Ordering::SeqCst);
    *proxy_state.port.lock().unwrap() = Some(port);
    *proxy_state.fingerprint.lock().unwrap() = fingerprint;
    *proxy_state.shutdown_tx.lock().unwrap() = Some(shutdown_tx);
    Ok(port)
}

#[tauri::command]
pub fn stop_proxy(proxy_state: State<'_, Arc<ProxyState>>) -> Result<(), String> {
    proxy_state.running.store(false, Ordering::SeqCst);
    *proxy_state.port.lock().unwrap() = None;
    // Send shutdown signal
    if let Some(tx) = proxy_state.shutdown_tx.lock().unwrap().take() {
        let _ = tx.send(());
    }
    Ok(())
}

#[tauri::command]
pub fn get_proxy_port(proxy_state: State<'_, Arc<ProxyState>>) -> Result<Option<u16>, String> {
    Ok(*proxy_state.port.lock().unwrap())
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
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
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
pub async fn open_dashboard(server_url: String, token: String, port: u16) -> Result<(), String> {
    if token.is_empty() {
        return open_browser(server_url);
    }

    let exchange_token = auth::create_exchange_token(&server_url, &token).await?;
    let base = server_url.trim_end_matches('/');
    let mut url = Url::parse(base).map_err(|e| e.to_string())?;
    url.set_path("/api/auth/exchange-token");
    {
        let mut pairs = url.query_pairs_mut();
        pairs.append_pair("exchangeToken", &exchange_token);
        pairs.append_pair("port", &port.to_string());
    }
    open_browser(url.to_string())
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
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut failures: u32 = 0;
            let mut last_failure_time: Option<std::time::Instant> = None;
            loop {
                if !cancel.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
                if !cancel.load(Ordering::SeqCst) {
                    break;
                }

                // Re-read token each cycle so session renewals are picked up
                let token = load_config().map(|c| c.token).unwrap_or_default();
                if token.is_empty() {
                    let _ = app_handle.emit("connection-lost", ());
                    break;
                }

                match auth::check_device_status(&server_url, &fingerprint, &token).await {
                    Ok(crate::auth::DeviceStatus::Revoked) => {
                        let _ = app_handle.emit("device-revoked", ());
                        app_handle.exit(0);
                        break;
                    }
                    Ok(crate::auth::DeviceStatus::Active)
                    | Ok(crate::auth::DeviceStatus::Pending) => {
                        failures = 0;
                        let _ = app_handle.emit("heartbeat-ok", ());
                    }
                    Ok(crate::auth::DeviceStatus::Error(ref reason))
                        if reason == "SESSION_INVALID" || reason == "FINGERPRINT_MISMATCH" =>
                    {
                        let _ = app_handle.emit("connection-lost", ());
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
            heartbeat_state.running.store(false, Ordering::SeqCst);
            *heartbeat_state.cancel.lock().unwrap() = None;
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
    proxy_state: State<'_, Arc<ProxyState>>,
    heartbeat_state: State<'_, Arc<HeartbeatState>>,
) -> Result<(), String> {
    proxy_state.running.store(false, Ordering::SeqCst);
    if let Some(tx) = proxy_state.shutdown_tx.lock().unwrap().take() {
        let _ = tx.send(());
    }
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
    let resp = client.get(&ping_url).send().await.map_err(|e| format!("connect failed: {}", e))?;
    let status = resp.status().as_u16();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
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
    let dir = crate::config::get_settings_dir();
    std::fs::create_dir_all(&dir).map_err(|e| format!("Failed to create dir: {}", e))?;
    let path = dir.join("device.key");
    // Backup existing key before overwriting
    if path.exists() {
        let backup = path.with_extension("key.bak");
        std::fs::copy(&path, &backup).map_err(|e| format!("Failed to backup: {}", e))?;
    }
    std::fs::write(&path, &data).map_err(|e| format!("Failed to write: {}", e))?;
    Ok(())
}
