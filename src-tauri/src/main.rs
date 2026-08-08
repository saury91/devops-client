#![windows_subsystem = "windows"]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::Manager;

use devops_client::{
    commands, fingerprint, i18n,
    state::{HeartbeatState, ProxyState},
};

fn main() {
    // Generate fingerprint on startup
    let _fp = fingerprint::get_or_create_fingerprint();

    let lang = i18n::detect_lang();

    let proxy_state = Arc::new(ProxyState {
        running: AtomicBool::new(false),
        port: Mutex::new(None),
        fingerprint: Mutex::new(String::new()),
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
        .plugin(tauri_plugin_shell::init())
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
            commands::start_heartbeat,
            commands::stop_heartbeat,
            commands::resize_window,
            commands::minimize_window,
            commands::hide_window,
            commands::start_drag,
            commands::quit_app,
            commands::get_device_info,
            commands::test_connection,
            commands::export_log_file,
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

            let open_item = match MenuItemBuilder::with_id("open", open_label).build(app) {
                Ok(item) => item,
                Err(e) => {
                    eprintln!("Failed to create menu item 'open': {}", e);
                    return Err(Box::new(e));
                }
            };
            let quit_item = match MenuItemBuilder::with_id("quit", quit_label).build(app) {
                Ok(item) => item,
                Err(e) => {
                    eprintln!("Failed to create menu item 'quit': {}", e);
                    return Err(Box::new(e));
                }
            };

            let menu = match MenuBuilder::new(app).item(&open_item).separator().item(&quit_item).build() {
                Ok(m) => m,
                Err(e) => {
                    eprintln!("Failed to build tray menu: {}", e);
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
                    eprintln!("Failed to build tray icon: {}", e);
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
