use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter, Manager, State};

use crate::auth;
use crate::config::{load_config, save_config, Config};

fn get_hardware_info() -> String {
    let mut info = serde_json::Map::new();
    info.insert("hostname".into(), serde_json::Value::String(
        hostname::get().map(|h| h.to_string_lossy().to_string()).unwrap_or_default()
    ));

    if cfg!(target_os = "macos") {
        if let Ok(out) = std::process::Command::new("sysctl")
            .args(["-n", "hw.model"]).output()
        {
            info.insert("model".into(), serde_json::Value::String(
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            ));
        }
        if let Ok(out) = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"]).output()
        {
            info.insert("cpu".into(), serde_json::Value::String(
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            ));
        }
        if let Ok(out) = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"]).output()
        {
            let bytes: u64 = String::from_utf8_lossy(&out.stdout).trim().parse().unwrap_or(0);
            info.insert("memory".into(), serde_json::Value::String(
                format!("{} GB", bytes / 1024 / 1024 / 1024)
            ));
        }
        if let Ok(out) = std::process::Command::new("sh")
            .args(["-c", "df -h / | tail -1 | awk '{print $2\", \"$4\" free\"}'"]).output()
        {
            info.insert("disk".into(), serde_json::Value::String(
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            ));
        }
    } else if cfg!(target_os = "windows") {
        if let Ok(out) = std::process::Command::new("wmic")
            .args(["computersystem", "get", "model"]).output()
        {
            let s = String::from_utf8_lossy(&out.stdout).to_string();
            let lines: Vec<&str> = s.lines().collect();
            if lines.len() > 1 { info.insert("model".into(), serde_json::Value::String(lines[1].trim().to_string())); }
        }
        if let Ok(out) = std::process::Command::new("wmic")
            .args(["cpu", "get", "name"]).output()
        {
            let s = String::from_utf8_lossy(&out.stdout).to_string();
            let lines: Vec<&str> = s.lines().collect();
            if lines.len() > 1 { info.insert("cpu".into(), serde_json::Value::String(lines[1].trim().to_string())); }
        }
    } else {
        if let Ok(out) = std::process::Command::new("sh")
            .args(["-c", "cat /proc/cpuinfo | grep 'model name' | head -1 | cut -d: -f2"]).output()
        {
            info.insert("cpu".into(), serde_json::Value::String(
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            ));
        }
        if let Ok(out) = std::process::Command::new("sh")
            .args(["-c", "free -h | grep Mem | awk '{print $2}'"]).output()
        {
            info.insert("memory".into(), serde_json::Value::String(
                String::from_utf8_lossy(&out.stdout).trim().to_string()
            ));
        }
    }

    serde_json::Value::Object(info).to_string()
}
use crate::fingerprint;
use crate::i18n::{self, Lang};
use crate::proxy;
use crate::state::{HeartbeatState, ProxyState};

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
    save_config(&config);
    Ok(())
}

#[tauri::command]
pub fn get_hostname() -> Result<String, String> {
    Ok(hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string()))
}

#[tauri::command]
pub fn get_os_info() -> serde_json::Value {
    serde_json::json!({
        "os": if cfg!(target_os = "macos") { "macOS" }
              else if cfg!(target_os = "windows") { "Windows" }
              else { "Linux" },
        "osVersion": std::env::consts::OS,
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
    let fp = fingerprint::get_or_create_fingerprint();
    let lang = i18n::detect_lang();
    let os = if cfg!(target_os = "macos") { "macOS" }
             else if cfg!(target_os = "windows") { "Windows" }
             else { "Linux" };
    let device_info = get_hardware_info();
    let result = auth::login_device(&server_url, &username, &password, &fp.value, &device_name,
                                     os, std::env::consts::OS, env!("CARGO_PKG_VERSION"), &device_info)
        .await
        .map_err(|e| i18n::t(lang, "login.connFailed").to_string() + ": " + &e)?;

    if result.code != 200 {
        return Err(result.msg);
    }

    let data = result
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

#[tauri::command]
pub async fn get_user_info(server_url: String, token: String) -> Result<serde_json::Value, String> {
    let resp = auth::get_user_info(&server_url, &token).await?;
    if resp.code != 200 {
        return Err(resp.msg);
    }
    let info = resp.data.ok_or_else(|| "用户信息为空".to_string())?;
    Ok(serde_json::json!({
        "id": info.id,
        "username": info.username.unwrap_or_default(),
        "nickname": info.nickname.unwrap_or_default(),
        "avatar": info.avatar.unwrap_or_default()
    }))
}

#[tauri::command]
pub async fn auto_login(server_url: String, fingerprint: String) -> Result<serde_json::Value, String> {
    let resp = auth::auto_login(&server_url, &fingerprint).await?;
    if resp.code != 200 {
        return Err(resp.msg);
    }
    let data = resp.data.ok_or_else(|| "自动登录响应为空".to_string())?;
    let token = data.token.unwrap_or_default();
    Ok(serde_json::json!({ "token": token }))
}

#[tauri::command]
pub async fn server_logout(server_url: String, token: String) -> Result<(), String> {
    let client = reqwest::Client::builder()
        .danger_accept_invalid_certs(false)
        .build()
        .map_err(|e| e.to_string())?;

    let base = server_url.trim_end_matches('/');
    let paths = ["/logout", "/api/auth/device-offline"];

    for path in paths {
        let url = format!("{}{}", base, path);
        let _ = client
            .post(&url)
            .header("X-Session-Id", &token)
            .send()
            .await;
    }

    Ok(())
}

#[tauri::command]
pub fn start_proxy(
    fingerprint: String,
    proxy_state: State<'_, Arc<ProxyState>>,
) -> Result<u16, String> {
    let (port, shutdown_tx) = proxy::start_proxy(fingerprint.clone())?;
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
    let result = if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(&url).spawn()
    } else if cfg!(target_os = "windows") {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &url])
            .spawn()
    } else {
        std::process::Command::new("xdg-open").arg(&url).spawn()
    };

    result.map_err(|e| format!("Failed to open browser: {}", e))?;
    Ok(())
}

#[tauri::command]
pub async fn open_dashboard(server_url: String, token: String, port: u16) -> Result<(), String> {
    if token.is_empty() {
        return open_browser(server_url);
    }

    let exchange_token = auth::create_exchange_token(&server_url, &token).await?;
    let url = format!(
        "{}/api/auth/exchange-token?exchangeToken={}&port={}",
        server_url.trim_end_matches('/'),
        exchange_token,
        port
    );
    open_browser(url)
}

#[tauri::command]
pub fn start_heartbeat(
    server_url: String,
    fingerprint: String,
    heartbeat_state: State<'_, Arc<HeartbeatState>>,
    app_handle: AppHandle,
) -> Result<(), String> {
    if heartbeat_state.running.load(Ordering::SeqCst) {
        return Ok(());
    }
    heartbeat_state.running.store(true, Ordering::SeqCst);

    let heartbeat_state = heartbeat_state.inner().clone();
    let token = load_config()
        .map(|c| c.token)
        .unwrap_or_default();

    let running = Arc::new(AtomicBool::new(true));
    let running_clone = running.clone();

    std::thread::spawn(move || {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async move {
            let mut failures: u32 = 0;
            loop {
                if !running_clone.load(Ordering::SeqCst) {
                    break;
                }
                tokio::time::sleep(Duration::from_secs(10)).await;
                if !running_clone.load(Ordering::SeqCst) {
                    break;
                }

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
                        failures = 0; // Reset on success
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
                        if failures >= 3 {
                            let _ = app_handle.emit("connection-lost", ());
                            break;
                        }
                        // Short retry delay on failure
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                    Err(_) => {
                        failures += 1;
                        if failures >= 3 {
                            let _ = app_handle.emit("connection-lost", ());
                            break;
                        }
                        // Short retry delay on failure
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
            }
            heartbeat_state.running.store(false, Ordering::SeqCst);
        });
    });

    Ok(())
}

#[tauri::command]
pub fn resize_window(app_handle: AppHandle, width: f64, height: f64) -> Result<(), String> {
    if let Some(window) = app_handle.get_webview_window("main") {
        window.set_size(tauri::LogicalSize::new(width, height))
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
pub fn quit_app(app_handle: AppHandle) -> Result<(), String> {
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
pub fn stop_heartbeat(
    heartbeat_state: State<'_, Arc<HeartbeatState>>,
) -> Result<(), String> {
    heartbeat_state.running.store(false, Ordering::SeqCst);
    Ok(())
}
