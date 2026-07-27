// Prevents an additional console window on Windows in release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

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
    });

    let heartbeat_state = Arc::new(HeartbeatState {
        running: AtomicBool::new(false),
    });

    tauri::Builder::default()
        .manage(proxy_state.clone())
        .manage(heartbeat_state.clone())
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
            commands::get_user_info,
            commands::do_login,
            commands::server_logout,
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

            let open_item = MenuItemBuilder::with_id("open", open_label)
                .build(app)
                .unwrap();
            let quit_item = MenuItemBuilder::with_id("quit", quit_label)
                .build(app)
                .unwrap();

            let menu = MenuBuilder::new(app)
                .item(&open_item)
                .separator()
                .item(&quit_item)
                .build()
                .unwrap();

            let proxy_state_clone = proxy_state.clone();
            let _tray = TrayIconBuilder::new()
                .icon(app.default_window_icon().unwrap().clone())
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
                .unwrap();

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
            let _ = (app_handle, event);
        });
}
