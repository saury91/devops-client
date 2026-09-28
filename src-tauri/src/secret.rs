//! 长期秘密（账号密码、会话 token）在系统凭据库中的存取。
//!
//! `config.json` 的加密密钥由机器 UUID 与登录用户名派生，而这两者对本地任意进程都
//! 可读 —— 文件里的密码因此只能算混淆，挡不住同机同用户的其它程序。把长期有效的
//! 秘密交给操作系统的凭据库，是唯一能真正抬高门槛的做法：钥匙串的访问控制绑定在应用
//! 身份上，而不是一个任何人都能推导出来的密钥。
//!
//! 平台差异：macOS（Keychain）与 Windows（Credential Manager）走系统凭据库；Linux
//! 没有实现（需要 libsecret 与 D-Bus 会话，无桌面会话时同样不可用），`is_supported()`
//! 返回 `false`，调用方回落到原有行为 —— 秘密留在加密的 `config.json` 里。
//!
//! 「存储位置降级」可接受，「秘密凭空丢失」不可接受：因此这里从不吞掉调用方的数据，
//! 写失败会如实返回错误，由调用方决定是否保留文件里的那一份。

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// 凭据库中的服务名，用 bundle identifier 以免与其它应用的条目冲突。
const SERVICE: &str = "io.github.devops-client";

/// 账号名沿用 `config.json` 里的字段名，便于排查时一一对应。
pub const ACCOUNT_TOKEN: &str = "token";
pub const ACCOUNT_PASSWORD: &str = "password";

/// 本平台是否支持系统凭据库。`false` 时调用方必须把秘密留在文件里。
pub fn is_supported() -> bool {
    cfg!(any(target_os = "macos", target_os = "windows"))
}

/// 读缓存：外层 `None` = 这个账号还没查过，内层 `None` = 查过且不存在。
type Cache = Mutex<HashMap<String, Option<String>>>;

fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// 命中缓存时返回 `Some(值)`；锁中毒（某个持锁线程 panic 过）时按未命中处理，只退化为
/// 再查一次凭据库，不会让进程卡住。
fn cached(account: &str) -> Option<Option<String>> {
    cache().lock().ok()?.get(account).cloned()
}

fn remember(account: &str, value: Option<String>) {
    if let Ok(mut guard) = cache().lock() {
        guard.insert(account.to_string(), value);
    }
}

fn forget(account: &str) {
    if let Ok(mut guard) = cache().lock() {
        guard.remove(account);
    }
}

/// 写入秘密。失败时清掉缓存 —— 留着旧值会让下一次 `load` 返回上一个密码。
pub fn store(account: &str, secret: &str) -> Result<(), String> {
    match imp::store(account, secret) {
        Ok(()) => {
            remember(account, Some(secret.to_string()));
            Ok(())
        }
        Err(e) => {
            forget(account);
            Err(e)
        }
    }
}

/// 读取秘密；`None` 表示凭据库里没有这一条。
///
/// 读失败（凭据库被锁、用户拒绝授权）也返回 `None`，但会落一条日志：调用方只关心
/// 「有没有拿到」，而排查的人需要知道到底是「没有」还是「没读到」。
pub fn load(account: &str) -> Option<String> {
    if let Some(hit) = cached(account) {
        return hit;
    }
    let value = match imp::load(account) {
        Ok(value) => value,
        Err(e) => {
            crate::config::log_error("secret", &format!("read '{}' failed: {}", account, e));
            None
        }
    };
    remember(account, value.clone());
    value
}

/// 删除秘密。条目本就不存在时视为成功 —— 调用方要的结果是「它没了」。
pub fn delete(account: &str) -> Result<(), String> {
    forget(account);
    match imp::delete(account) {
        Ok(()) => Ok(()),
        Err(e) => {
            crate::config::log_error("secret", &format!("delete '{}' failed: {}", account, e));
            Err(e)
        }
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
mod imp {
    use keyring::{Entry, Error as KeyringError};

    fn entry(account: &str) -> Result<Entry, String> {
        Entry::new(super::SERVICE, account)
            .map_err(|e| format!("credential store unavailable: {}", e))
    }

    pub fn store(account: &str, secret: &str) -> Result<(), String> {
        entry(account)?
            .set_password(secret)
            .map_err(|e| format!("credential store write failed: {}", e))
    }

    /// `Ok(None)` = 条目不存在。首次运行、用户主动清过都会走到这里，属于正常路径，
    /// 不能当错误报上去 —— 否则每次全新安装都会在日志里留一条假告警。
    pub fn load(account: &str) -> Result<Option<String>, String> {
        match entry(account)?.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(KeyringError::NoEntry) => Ok(None),
            Err(e) => Err(format!("credential store read failed: {}", e)),
        }
    }

    pub fn delete(account: &str) -> Result<(), String> {
        match entry(account)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(KeyringError::NoEntry) => Ok(()),
            Err(e) => Err(format!("credential store delete failed: {}", e)),
        }
    }
}

/// 其余平台（Linux、BSD）：没有可用的系统凭据库。这里不静默成功 —— 静默成功会让
/// 调用方以为秘密已经安全移走，从而把 `config.json` 里的那一份擦掉，秘密就真没了。
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
mod imp {
    const UNSUPPORTED: &str = "no system credential store on this platform";

    pub fn store(_account: &str, _secret: &str) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }

    pub fn load(_account: &str) -> Result<Option<String>, String> {
        Err(UNSUPPORTED.to_string())
    }

    pub fn delete(_account: &str) -> Result<(), String> {
        Err(UNSUPPORTED.to_string())
    }
}
