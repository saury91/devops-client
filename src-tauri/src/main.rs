#![windows_subsystem = "windows"]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::Manager;

use devops_client::{
    commands, config, i18n,
    state::{HeartbeatState, ProxyState},
};

fn main() {
    // 必须最先装：托盘构建失败、后台线程 panic 这些都发生在后面，而 Windows 上没有控制台，
    // 不落盘就等于什么都没说。装在这里可以保证此后任何一次 panic 都能被「导出日志」带回来。
    config::install_panic_hook();

    // 这里原本每次都调一次 `get_or_create_fingerprint()` 并把返回值丢掉，等于启动时白跑一遍
    // Argon2 解密和机器识别。设备密钥是懒创建的，前端加载时会走 get_fingerprint 命令。
    let lang = i18n::detect_lang();

    let proxy_state = Arc::new(ProxyState {
        running: AtomicBool::new(false),
        port: Mutex::new(None),
        shutdown_tx: Mutex::new(None),
        start_lock: Mutex::new(()),
    });

    let heartbeat_state = Arc::new(HeartbeatState {
        running: AtomicBool::new(false),
        cancel: Mutex::new(None),
        start_lock: Mutex::new(()),
    });

    tauri::Builder::default()
        .manage(proxy_state.clone())
        .manage(heartbeat_state.clone())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.set_focus();
            }
        }))
        .invoke_handler(tauri::generate_handler![
            commands::get_lang,
            commands::get_fingerprint,
            commands::load_config_cmd,
            commands::save_config_cmd,
            commands::get_hostname,
            commands::get_os_info,
            commands::get_user_info,
            commands::do_login,
            commands::server_logout,
            commands::change_password,
            commands::auto_login,
            commands::start_proxy,
            commands::stop_proxy,
            commands::get_proxy_port,
            commands::open_browser,
            commands::open_dashboard,
            commands::get_dashboard_url,
            commands::start_heartbeat,
            commands::stop_heartbeat,
            commands::get_cert_status,
            commands::install_device_cert,
            commands::resize_window,
            commands::minimize_window,
            commands::hide_window,
            commands::start_drag,
            commands::quit_app,
            commands::get_device_info,
            commands::test_connection,
            commands::export_log_file,
            commands::read_error_log,
            commands::export_device_key,
            commands::import_device_key,
        ])
        .setup(move |app| {
            use tauri::menu::{MenuBuilder, MenuItemBuilder};
            use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

            let open_label = i18n::t(lang, "tray.open");
            let quit_label = i18n::t(lang, "tray.quit");
            let tooltip = i18n::t(lang, "tray.tooltip");
            let window_title = i18n::t(lang, "window.title");

            // Set window title
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title(window_title);
            }

            // 托盘相关失败都会带 Err 逃出 setup，最终变成 build() 的 panic：没有托盘就等于没有
            // 常驻入口，失败必须中止启动。但 Windows 上没有控制台（本文件第一行就是
            // `windows_subsystem = "windows"`），panic 的内容没人看得见，所以先落一份日志。
            let open_item = match MenuItemBuilder::with_id("open", open_label).build(app) {
                Ok(item) => item,
                Err(e) => {
                    config::log_error("tray", &format!("failed to create menu item 'open': {}", e));
                    return Err(Box::new(e));
                }
            };
            let quit_item = match MenuItemBuilder::with_id("quit", quit_label).build(app) {
                Ok(item) => item,
                Err(e) => {
                    config::log_error("tray", &format!("failed to create menu item 'quit': {}", e));
                    return Err(Box::new(e));
                }
            };

            let menu = match MenuBuilder::new(app)
                .item(&open_item)
                .separator()
                .item(&quit_item)
                .build()
            {
                Ok(m) => m,
                Err(e) => {
                    config::log_error("tray", &format!("failed to build tray menu: {}", e));
                    return Err(Box::new(e));
                }
            };

            let proxy_state_clone = proxy_state.clone();
            // macOS 菜单栏使用白色版图标（与其它菜单栏图标风格一致）；其它平台与 Dock 应用图标仍用彩色
            #[cfg(target_os = "macos")]
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/tray-white.png"))
                .map_err(|e| format!("failed to load tray icon: {}", e))?;
            #[cfg(not(target_os = "macos"))]
            let icon = app
                .default_window_icon()
                .ok_or_else(|| "missing default window icon".to_string())?
                .clone();

            let _tray = match TrayIconBuilder::new()
                .icon(icon)
                .tooltip(tooltip)
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "open" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
                        proxy_state_clone.running.store(false, Ordering::SeqCst);
                        if let Some(tx) = proxy_state_clone.shutdown_tx.lock().unwrap().take() {
                            let _ = tx.send(());
                        }
                        let hb = app.state::<Arc<HeartbeatState>>();
                        hb.running.store(false, Ordering::SeqCst);
                        if let Some(cancel) = hb.cancel.lock().unwrap().take() {
                            cancel.store(false, Ordering::SeqCst);
                        }
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        if let Some(window) = tray.app_handle().get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                })
                .build(app)
            {
                Ok(t) => t,
                Err(e) => {
                    config::log_error("tray", &format!("failed to build tray icon: {}", e));
                    return Err(Box::new(e));
                }
            };

            Ok(())
        })
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen { .. } = event {
                if let Some(window) = app_handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = app_handle;
                let _ = event;
            }
        });
}
