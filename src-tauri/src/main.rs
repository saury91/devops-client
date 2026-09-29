#![windows_subsystem = "windows"]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use tauri::Manager;

use devops_client::{commands, config, i18n, state::HeartbeatState};

fn main() {
    // 必须最先装：托盘构建失败、后台线程 panic 这些都发生在后面，而 Windows 上没有控制台，
    // 不落盘就等于什么都没说。装在这里可以保证此后任何一次 panic 都能被「导出日志」带回来。
    config::install_panic_hook();

    // 这里原本每次都调一次 `get_or_create_fingerprint()` 并把返回值丢掉，等于启动时白跑一遍
    // Argon2 解密和机器识别。设备密钥是懒创建的，前端加载时会走 get_fingerprint 命令。
    let lang = i18n::detect_lang();

    let heartbeat_state = Arc::new(HeartbeatState {
        running: AtomicBool::new(false),
        cancel: Mutex::new(None),
        start_lock: Mutex::new(()),
    });

    tauri::Builder::default()
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
            commands::open_browser,
            commands::open_dashboard,
            commands::get_dashboard_url,
            commands::start_heartbeat,
            commands::stop_heartbeat,
            commands::is_dev_build,
            commands::show_main_window,
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
            // 调试构建在托盘提示和窗口标题上都标出来，免得和正式安装的那一份混淆。
            let dev_suffix = if cfg!(debug_assertions) { " [DEV]" } else { "" };
            let tooltip = format!("{}{}", i18n::t(lang, "tray.tooltip"), dev_suffix);
            let window_title = format!("{}{}", i18n::t(lang, "window.title"), dev_suffix);

            // Set window title
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.set_title(&window_title);
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
                .tooltip(&tooltip)
                .menu(&menu)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "open" => {
                        if let Some(window) = app.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.set_focus();
                        }
                    }
                    "quit" => {
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
            // 退出前最后一步：尽力通知服务端本机已离线，让服务端立刻失效这台设备的桌面端与
            // 浏览器端会话，而不是等在场标记的 TTL（默认 300 秒）自然过期。托盘「退出」、面板的
            // 退出按钮、macOS 的 Cmd+Q 最终都经 app.exit() 触发本事件，挂这一个出口即可全覆盖。
            // 内部带 2 秒超时且失败只记日志，不会把退出流程卡住。
            if let tauri::RunEvent::ExitRequested { .. } = event {
                commands::notify_device_offline();
            }
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
            }
        });
}
