# DevOps Client

`fw-devops` 平台的 Tauri v2 桌面设备认证代理。通过本地 HTTPS 服务、设备指纹绑定和客户端心跳，为 Web 端提供"仅在当前机器可用"的安全会话管理，并支持系统托盘常驻与中/英国际化。

## 工作原理

```
用户登录 → 服务端注册/校验设备指纹 → 客户端启动本地 HTTPS 代理
                                              ↓
浏览器端 → fetch('http://127.0.0.1:{port}/ping')
  → 成功 → 请求来自受信任机器 → 保持 Web 会话可用
  → 失败 → Cookie 被复制到其他机器或客户端已关闭 → 失效会话
```

客户端同时维护与服务端的心跳：

```
每 10 秒 POST /api/auth/device-status
  → fingerprint 与 session 匹配 → 续期同指纹所有 session
  → fingerprint 不匹配 → 立即失效当前 session 并踢掉该设备用户
  → 连续 3 次失败 → 触发 connection-lost，客户端返回登录页
```

---

## 功能特性

- **设备绑定安全** — ED25519 + SHA256 设备指纹，会话与物理机器绑定
- **设备审批** — 新设备首次登录可自动审批（首台）或进入管理员审批队列
- **安全打开工作台** — 通过一次性 exchange-token 兑换独立浏览器 session
- **客户端心跳** — 自动续期、设备撤销检测、三次失败退出
- **国际化** — 中/英双语，根据系统 locale 自动检测
- **系统托盘** — 关闭窗口后常驻后台，托盘菜单可快速打开/退出
- **跨平台** — macOS（ARM64 / x64）、Windows（x64）、Linux（x64）
- **自签 TLS** — 自动生成 CA 与 localhost 证书，并安装到系统信任存储

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
├── tauri.conf.json             # 窗口 420×560、托盘、CSP
└── src/
    ├── main.rs                 # 入口：窗口、托盘、生命周期
    ├── lib.rs                  # 模块声明
    ├── commands.rs             # 全部 Tauri IPC 命令
    ├── state.rs                # ProxyState, HeartbeatState
    ├── config.rs               # Config 加载/保存
    ├── fingerprint.rs          # ED25519 + SHA-256 设备指纹
    ├── crypto.rs               # AES-256-GCM 本地加密
    ├── cert.rs                 # 自签 CA + localhost TLS
    ├── proxy.rs                # Axum HTTPS 服务器（/ping）
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
├── config.json         # 缓存：serverUrl、token、fingerprint、loginAt 等
└── certs/
    ├── ca.pem          # 自签 CA（首次启动时安装到系统信任存储）
    ├── localhost.pem   # 127.0.0.1 证书
    └── localhost-key.pem
```

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
| `server_logout` | 调用服务端登出与 device-offline |
| `start_proxy` | 启动本地 HTTPS 代理 |
| `stop_proxy` | 停止本地 HTTPS 代理 |
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

系统语言从环境变量（`LANG`、`AppleLocale`）自动检测，默认回退英文。

| 语言 | 代码 |
|------|------|
| English | `en` |
| 中文 | `zh` |

新增语言：
1. 创建 `src/locales/{code}.json`
2. 在 `src-tauri/src/i18n.rs` 中添加 key
3. 更新 `detect_lang()` 中的检测逻辑

---

## 架构说明

### 认证流程

1. **首次/手动登录**：`do_login` → `/api/auth/login-device`
2. **自动登录**：启动时若缓存了 fingerprint，调用 `/api/auth/auto-login` 换取新 token
3. **打开工作台**：`open_dashboard` → `create-exchange-token` → 系统浏览器打开 `exchange-token` 端点
4. **心跳续期**：`start_heartbeat` 每 10 秒 POST `/api/auth/device-status`

### 心跳状态机

```
Active / Pending  → 失败计数清零
Revoked           → 发送 device-revoked，退出应用
SESSION_INVALID / FINGERPRINT_MISMATCH → 立即 connection-lost
其他 Error / NotFound / 网络错误      → 失败计数 +1，达到 3 次后 connection-lost
```

### 指纹算法

```
raw = systemUUID + "/" + hostname + "/" + base64url(ed25519_pubkey)
fingerprint = SHA256(raw) → 64 位小写 hex
```

身份标识持久化在 `~/.devops-client/device.key`。

---

## 平台支持

| 平台 | 本地开发 | CI |
|------|----------|----|
| macOS ARM64 | `just build` | ✓ |
| macOS x86_64 | `just build-mac-x64` | ✓ |
| Windows x86_64 | — | ✓ |
| Linux x86_64 | — | ✓ |

---

## 开发规范

- Rust 代码遵循 `cargo fmt` 与 `cargo clippy -- -D warnings`
- 前端无打包工具，所有 JS 模块通过全局变量暴露
- 新增 IPC 命令需在 `commands.rs` 实现并在前端 `api.js` 封装
- 不要在前端或 Rust 中硬编码服务器地址、密钥等敏感信息

---

## License

MIT
