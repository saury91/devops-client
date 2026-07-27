use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Lang {
    En,
    Zh,
}

pub fn detect_lang() -> Lang {
    // Check environment variables for locale
    for var in &["LANG", "LC_ALL", "LC_MESSAGES"] {
        if let Ok(val) = std::env::var(var) {
            let lower = val.to_lowercase();
            if lower.starts_with("zh") || lower.contains("zh_cn") {
                return Lang::Zh;
            }
        }
    }

    // macOS: check AppleLocale
    if let Ok(val) = std::env::var("AppleLocale") {
        if val.starts_with("zh") {
            return Lang::Zh;
        }
    }

    #[cfg(target_os = "windows")]
    {
        if let Ok(output) = std::process::Command::new("powershell")
            .args(["-Command", "(Get-Culture).Name"])
            .output()
        {
            let s = String::from_utf8_lossy(&output.stdout);
            if s.trim().starts_with("zh") {
                return Lang::Zh;
            }
        }
    }

    Lang::Zh // Default to Chinese
}

pub fn t(lang: Lang, key: &str) -> &str {
    let map: &HashMap<&str, &str> = match lang {
        Lang::En => &EN,
        Lang::Zh => &ZH,
    };
    map.get(key).copied().unwrap_or(key)
}

macro_rules! i18n_map {
    ($( $key:expr => { en: $en:expr, zh: $zh:expr } ),* $(,)?) => {
        fn en_map() -> HashMap<&'static str, &'static str> {
            let mut m = HashMap::new();
            $( m.insert($key, $en); )*
            m
        }
        fn zh_map() -> HashMap<&'static str, &'static str> {
            let mut m = HashMap::new();
            $( m.insert($key, $zh); )*
            m
        }
        static EN: std::sync::LazyLock<HashMap<&'static str, &'static str>> =
            std::sync::LazyLock::new(en_map);
        static ZH: std::sync::LazyLock<HashMap<&'static str, &'static str>> =
            std::sync::LazyLock::new(zh_map);
    };
}

i18n_map! {
    "window.title" => { en: "DevOps", zh: "DevOps" },
    "tray.tooltip" => { en: "DevOps Client", zh: "DevOps 客户端" },
    "tray.open" => { en: "Open Panel", zh: "打开面板" },
    "tray.quit" => { en: "Quit", zh: "退出" },

    "login.title" => { en: "DevOps", zh: "DevOps" },
    "login.subtitle" => { en: "DESKTOP IDENTITY", zh: "桌面身份验证" },
    "login.username" => { en: "Username", zh: "用户名" },
    "login.password" => { en: "Password", zh: "密码" },

    "login.signIn" => { en: "SIGN IN", zh: "登 录" },
    "login.signingIn" => { en: "SIGNING IN...", zh: "登录中..." },
    "login.quit" => { en: "Quit", zh: "退出" },
    "login.fillAll" => { en: "Please fill in all fields", zh: "请填写完整信息" },
    "login.pending" => { en: "New device needs admin approval", zh: "新设备需要管理员审批" },
    "login.failed" => { en: "Login failed", zh: "登录失败" },
    "login.connFailed" => { en: "Connection failed", zh: "连接失败" },

    "panel.title" => { en: "DevOps", zh: "DevOps" },
    "panel.connected" => { en: "CONNECTED", zh: "已连接" },
    "panel.autoConnected" => { en: "DEVICE RECOGNIZED", zh: "设备已识别" },
    "panel.server" => { en: "Server", zh: "服务端" },
    "panel.deviceId" => { en: "Device ID", zh: "设备 ID" },
    "panel.localPort" => { en: "Port", zh: "端口" },
    "panel.openDashboard" => { en: "Open Dashboard", zh: "打开工作台" },
    "panel.quit" => { en: "Logout", zh: "退出登录" },

    "settings.title" => { en: "Settings", zh: "设置" },
    "settings.serverUrl" => { en: "Server URL", zh: "服务器地址" },
    "settings.language" => { en: "Language", zh: "语言" },
    "settings.langZh" => { en: "中文", zh: "中文" },
    "settings.langEn" => { en: "English", zh: "English" },
    "settings.save" => { en: "Save", zh: "保存" },
    "settings.saved" => { en: "Saved", zh: "已保存" },

    "error.serverError" => { en: "Server returned an error", zh: "服务器返回异常" },
    "error.revoked" => { en: "Device has been revoked by admin.", zh: "设备已被管理员撤销。" },
    "error.noServerUrl" => { en: "Please configure server URL in settings", zh: "请先在设置中配置服务器地址" },
    "error.badCredentials" => { en: "Invalid username or password", zh: "用户名或密码错误" },
    "error.accountLocked" => { en: "Account is locked", zh: "账号已被锁定" },
    "error.networkError" => { en: "Network error — please check your connection", zh: "网络错误 — 请检查网络连接" },
    "error.sessionExpired" => { en: "Session expired — please login again", zh: "会话已过期 — 请重新登录" },
    "error.connectionLost" => { en: "Connection to server lost", zh: "与服务器的连接已断开" },

    "panel.settings" => { en: "Settings", zh: "设置" },
    "panel.deviceInfo" => { en: "Device Info", zh: "设备信息" },
    "panel.deviceOs" => { en: "OS", zh: "系统" },
    "panel.deviceSerial" => { en: "S/N", zh: "序列号" },
    "panel.deviceModel" => { en: "Model", zh: "型号" },
    "panel.deviceCpu" => { en: "CPU", zh: "处理器" },
    "panel.deviceGpu" => { en: "GPU", zh: "显卡" },
    "panel.deviceMemory" => { en: "Memory", zh: "内存" },
    "panel.deviceDisk" => { en: "Disk", zh: "磁盘" },
    "panel.noDeviceInfo" => { en: "No hardware info available", zh: "暂无硬件信息" },
    "panel.diagnostics" => { en: "Diagnostics", zh: "诊断信息" },
    "panel.diagPort" => { en: "Proxy Port", zh: "代理端口" },
    "panel.diagLatency" => { en: "Latency", zh: "延迟" },
    "panel.diagLastHb" => { en: "Last HB", zh: "上次心跳" },
    "panel.diagFingerprint" => { en: "Fingerprint", zh: "指纹" },
    "panel.exportLog" => { en: "Export", zh: "导出" },

    "settings.testConn" => { en: "Test", zh: "测试" },
    "settings.testing" => { en: "Testing...", zh: "测试中..." },
    "settings.connOk" => { en: "Connected", zh: "连接成功" },
    "settings.connFail" => { en: "Failed", zh: "连接失败" },
    "settings.urlHistory" => { en: "History", zh: "历史" },
}
