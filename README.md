# DevOps Client

基于 Tauri v2 的桌面设备认证代理。客户端通过设备指纹绑定、本地 HTTP 代理和心跳保活，与配套 Web 后端协同实现“仅在当前机器可用”的安全访问。

## 客户端流程

```
登录/自动登录
  → 服务端返回 session token
  → 客户端加密保存配置（serverUrl、token、loginAt 等）
  → 启动本地 HTTP 代理（127.0.0.1 随机端口，/ping）
  → 启动 10 秒心跳
  → 进入已连接面板
```

点击面板“工作台”圆球时：

```
申请 exchange-token
  → 系统浏览器打开 /api/auth/exchange-token?...
  → 服务端写入浏览器专属 cookie
  → 浏览器加载 Web 工作台
```

本地 HTTP 代理仅监听 `127.0.0.1`，不对外暴露，用于让浏览器验证桌面 Agent 真实存在。

心跳根据服务端返回状态执行不同动作：

- `Active` / `Pending`：失败计数清零
- `Revoked`：提示“设备已被撤销”并退出应用
- `SESSION_INVALID` / `FINGERPRINT_MISMATCH`：立即返回登录页
- 其他错误/网络异常：累计失败，达到 3 次后返回登录页

---

## 功能特性

- **设备绑定** — ED25519 + SHA256 生成设备指纹
- **设备审批** — 新设备首次登录可自动审批（首台）或进入审批队列
- **安全打开工作台** — 通过一次性 exchange-token 兑换浏览器 session
- **客户端心跳** — 自动续期、撤销检测、三次失败回登录页
- **国际化** — 中/英双语，根据系统 locale 自动检测
- **系统托盘** — 关闭窗口后常驻后台，托盘菜单可快速打开/退出
- **跨平台** — macOS（ARM64 / x64）、Windows（x64）、Linux（x64）

---

## 环境要求

| 平台 | 依赖 |
|------|------|
| macOS | Xcode Command Line Tools |
| Linux | `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev` |
| Windows | Microsoft Visual Studio C++ Build Tools |
| 全部 | Rust 1.80+，Node.js 20+，pnpm，just |

---

## 快速开始

```bash
# 1. 安装依赖
just install

# 2. 开发模式（热重载）
just dev

# 3. 仅类型检查
just check

# 4. 代码格式化与 lint
just fmt
just lint

# 5. 本地构建 macOS ARM64 DMG
just build
```

构建产物：

```
src-tauri/target/aarch64-apple-darwin/release/bundle/
├── macos/DevOps Client.app
└── dmg/DevOps Client_0.1.0_aarch64.dmg
```

---

## 项目结构

```
src/                            # 前端（WebView UI）
├── index.html                  # Shell 页面（data-i18n 属性）
├── css/styles.css              # 深色主题
├── fonts/                      # Inter / JetBrains Mono 本地字体
├── js/
│   ├── app.js                  # 入口、视图切换、启动/退出
│   ├── api.js                  # Tauri IPC 封装
│   ├── i18n.js                 # 前端 i18n 引擎
│   ├── avatar.js               # 默认头像 SVG 生成
│   └── views/
│       ├── login.js            # 登录表单逻辑
│       ├── panel.js            # 已连接面板（用户信息、日志、工作台入口）
│       ├── background.js       # 动态背景
│       ├── wave.js             # 监视器圆球水波纹动画
│       └── settings.js         # 设置页
└── locales/
    ├── en.json                 # 英文翻译
    └── zh.json                 # 中文翻译

src-tauri/                      # Tauri Rust 后端
├── Cargo.toml
├── tauri.conf.json             # 窗口、托盘、CSP、打包目标
└── src/
    ├── main.rs                 # 入口：窗口、托盘、生命周期
    ├── lib.rs                  # 模块声明
    ├── commands.rs             # 全部 Tauri IPC 命令
    ├── state.rs                # ProxyState, HeartbeatState
    ├── config.rs               # Config 加载/保存（AES-256-GCM 加密）
    ├── fingerprint.rs          # ED25519 + SHA-256 设备指纹
    ├── crypto.rs               # AES-256-GCM 本地加密
    ├── proxy.rs                # Axum HTTP 本地代理（/ping）
    ├── auth.rs                 # Reqwest HTTP 客户端（登录/心跳/换 token/用户信息）
    ├── error.rs                # AppError（带 i18n 消息）
    └── i18n.rs                 # 语言检测 + 翻译表
```

---

## 配置与数据

所有数据存储在 `~/.devops-client/`：

```
.devops-client/
├── device.key          # ED25519 密钥对（JSON: seed + pub）
└── config.json         # 加密缓存：serverUrl、token、fingerprint、loginAt 等
```

`config.json` 使用 AES-256-GCM 加密存储，不是明文 JSON，请勿手动编辑。首次写入时由 `crypto.rs` 根据机器 UUID 与用户名派生密钥。

---

## IPC 命令

| 命令 | 说明 |
|------|------|
| `get_lang` | 检测系统语言 |
| `get_fingerprint` | 获取设备指纹 |
| `load_config_cmd` | 加载本地缓存配置 |
| `save_config_cmd` | 保存配置到本地 |
| `get_hostname` | 获取 OS 主机名 |
| `get_os_info` | 获取 OS、OS 版本、客户端版本 |
| `do_login` | 调用 `/api/auth/login-device` 登录 |
| `auto_login` | 使用 fingerprint 调用 `/api/auth/auto-login` |
| `get_user_info` | 获取当前登录用户信息 |
| `server_logout` | 调用服务端登出 |
| `start_proxy` | 启动本地 HTTP 代理 |
| `stop_proxy` | 停止本地 HTTP 代理 |
| `get_proxy_port` | 获取当前代理端口 |
| `open_browser` | 使用系统默认浏览器打开 URL |
| `open_dashboard` | 申请 exchange-token 并打开工作台 |
| `start_heartbeat` | 启动 10 秒心跳循环 |
| `stop_heartbeat` | 停止心跳循环 |
| `resize_window` | 调整窗口大小 |
| `minimize_window` | 最小化窗口 |
| `hide_window` | 隐藏窗口到托盘 |
| `quit_app` | 退出应用 |
| `start_drag` | 开始窗口拖拽 |

---

## 国际化

系统语言从环境变量（`LANG`、`AppleLocale`）自动检测，默认回退中文。

| 语言 | 代码 |
|------|------|
| English | `en` |
| 中文 | `zh` |

新增语言：
1. 创建 `src/locales/{code}.json`
2. 在 `src-tauri/src/i18n.rs` 中添加 key
3. 更新 `detect_lang()` 中的检测逻辑

---

## 平台支持

| 平台 | 本地开发 | CI |
|------|----------|----|
| macOS ARM64 | `just build` | ✓ |
| macOS x86_64 | `just build-mac-x64` | ✓ |
| Windows x86_64 | — | ✓ |
| Linux x86_64 | — | ✓ |

`tauri.conf.json` 中 `bundle.targets` 已配置为 `"all"`，各平台 CI 会自动构建当前平台支持的所有安装包格式。

---

## 常见问题

### macOS 更新图标后仍显示旧图标

macOS 会缓存应用图标。替换 `src-tauri/icons/` 并重新构建后，若 Dock/启动台仍显示旧图标，可执行：

```bash
# 1. 移除图标缓存
rm -rf /private/var/folders/*/*/*/com.apple.dock.iconcache
rm -rf /private/var/folders/*/*/*/com.apple.iconservices.store

# 2. 重置 Dock 与 Finder
killall Dock
killall Finder
```

---

## 开发规范

- Rust 代码遵循 `cargo fmt` 与 `cargo clippy -- -D warnings`
- 前端无打包工具，所有 JS 模块通过全局变量暴露
- 新增 IPC 命令需在 `commands.rs` 实现并在前端 `api.js` 封装
- 不要在前端或 Rust 中硬编码服务器地址、密钥等敏感信息

---

## License

MIT
