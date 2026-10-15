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

            // 窗口在代码里按平台创建，而不是写在 tauri.conf.json 里。原因：两平台拿不到
            // 同一种可用的窗口形态，而形态只能在创建时决定（transparent 没有运行时 setter）。
            //
            //   · macOS —— 必须走 App Store 合规路径：保留系统边框 + 隐藏标题栏外观。
            //     圆角和阴影由 NSWindow 自己给，全是公开 AppKit API。
            //     而"透明窗口"依赖 macOS 私有 API（配置里的 transparent + macOSPrivateApi），
            //     一旦用了就无法上架 App Store，且无法在运行时关掉，所以不能作为默认形态。
            //
            //   · Windows —— 反过来不能用系统边框：无边框状态下系统不给圆角
            //     （Win11 只有开了 shadow 才给原生圆角，但会附赠一条 1px 白边）。
            //     所以走透明窗口，圆角由 CSS 画出来。
            // 原生窗口标题：macOS 必须置空。Overlay 风格下系统仍会把标题文字居中画在
            // 内容上层，跟前端自绘的标题（AUTO_TERMINAL + DEV 徽标）叠成重影；
            // 这个 tao 版本没有提供隐藏标题文字的开关（setTitleVisibility 没暴露），
            // 所以直接不给标题 —— 界面标题已经由 HTML 画了，任务栏/Dock 显示的是应用名。
            #[cfg(target_os = "macos")]
            let native_title: &str = "";
            #[cfg(not(target_os = "macos"))]
            let native_title: &str = &window_title;
            #[cfg(target_os = "macos")]
            let _ = &window_title;

            let mut builder = tauri::WebviewWindowBuilder::new(
                app,
                "main",
                tauri::WebviewUrl::App("index.html".into()),
            )
            .title(native_title)
            .inner_size(360.0, 320.0)
            .resizable(false)
            .visible(false)
            .center();

            #[cfg(target_os = "macos")]
            {
                builder = builder
                    .decorations(true)
                    // Overlay：内容铺满整个窗口，红绿灯由系统直接叠在内容左上角
                    // （VS Code / Slack 同款做法）。Transparent 会把内容挤到标题栏下面，
                    // 出现一条单独的标题栏带，和自绘标题栏叠两层，不要用。
                    .title_bar_style(tauri::TitleBarStyle::Overlay)
                    .shadow(true);
            }
            #[cfg(not(target_os = "macos"))]
            {
                builder = builder.decorations(false).transparent(true).shadow(false);
            }

            if let Err(e) = builder.build() {
                config::log_error("window", &format!("failed to create main window: {}", e));
                return Err(Box::new(e));
            }

            // 隐藏绿色缩放按钮：窗口固定尺寸（resizable:false），该按钮被系统禁用后
            // 仍渲染成一个黑色圆点悬在红绿灯组里，既不可用又难看。
            // standardWindowButton + setHidden 是公开 AppKit API，不影响上架。
            #[cfg(target_os = "macos")]
            if let Some(window) = app.get_webview_window("main") {
                if let Ok(ptr) = window.ns_window() {
                    use objc2_app_kit::{NSWindow, NSWindowButton};
                    let ns_win = ptr as *mut NSWindow;
                    if !ns_win.is_null() {
                        unsafe {
                            if let Some(btn) =
                                (*ns_win).standardWindowButton(NSWindowButton::ZoomButton)
                            {
                                btn.setHidden(true);
                            }
                        }
                    }
                }
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

            // macOS 菜单栏用模板图标（纯黑 + alpha），由系统按明暗主题自动着色：
            // 硬编码白色图标在浅色菜单栏下会直接消失，模板图两个主题都可见。
            // 其它平台仍用彩色应用图标（Windows 托盘 / Linux 面板都吃彩色）。
            #[cfg(target_os = "macos")]
            let icon =
                tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))
                    .map_err(|e| format!("failed to load tray icon: {}", e))?;
            #[cfg(not(target_os = "macos"))]
            let icon = app
                .default_window_icon()
                .ok_or_else(|| "missing default window icon".to_string())?
                .clone();

            #[allow(unused_mut)]
            let mut tray_builder = TrayIconBuilder::new()
                .icon(icon)
                .tooltip(&tooltip)
                .menu(&menu);
            #[cfg(target_os = "macos")]
            {
                tray_builder = tray_builder.icon_as_template(true);
            }
            let _tray = match tray_builder
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
