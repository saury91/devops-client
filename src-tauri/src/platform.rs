/// Platform-specific system information: UUID, serial, OS version.
/// Shared by fingerprint, crypto, and commands modules to avoid duplication.

pub fn system_uuid() -> String {
    #[cfg(target_os = "macos")]
    {
        if let Ok(o) = std::process::Command::new("sh")
            .args([
                "-c",
                "system_profiler SPHardwareDataType 2>/dev/null | awk '/Hardware UUID/{print $NF}'",
            ])
            .output()
        {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !s.is_empty() && s != "unknown" {
                return s;
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x08000000;

        // Try MachineGuid first (stable per OS install)
        if let Ok(o) = std::process::Command::new("reg")
            .args([
                "query",
                r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Cryptography",
                "/v",
                "MachineGuid",
            ])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            let s = String::from_utf8_lossy(&o.stdout);
            if let Some(line) = s.lines().find(|l| l.contains("REG_SZ")) {
                let guid = line.split("REG_SZ").nth(1).unwrap_or("").trim().to_string();
                if !guid.is_empty() {
                    return guid;
                }
            }
        }
        // Fallback to wmic
        if let Ok(o) = std::process::Command::new("wmic")
            .args(["csproduct", "get", "uuid"])
            .creation_flags(CREATE_NO_WINDOW)
            .output()
        {
            let s = String::from_utf8_lossy(&o.stdout);
            let uuid = s.lines().nth(1).unwrap_or("").trim().to_string();
            if !uuid.is_empty() {
                return uuid;
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        // /etc/machine-id is the most stable identifier on systemd Linux
        if let Ok(id) = std::fs::read_to_string("/etc/machine-id") {
            let id = id.trim().to_string();
            if !id.is_empty() && id.len() >= 32 {
                return id;
            }
        }
        // Fallback: DMI product UUID
        if let Ok(o) = std::process::Command::new("sh")
            .args([
                "-c",
                "cat /sys/class/dmi/id/product_uuid 2>/dev/null || dmidecode -s system-uuid 2>/dev/null",
            ])
            .output()
        {
            let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if !s.is_empty() {
                return s;
            }
        }
    }

    "unknown".to_string()
}

pub fn hostname() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_default()
}

pub fn os_name() -> &'static str {
    if cfg!(target_os = "macos") {
        "macOS"
    } else if cfg!(target_os = "windows") {
        "Windows"
    } else {
        "Linux"
    }
}
