# CLAUDE.md

DevOps Client — Tauri v2 桌面设备认证代理。作为 Web 后端服务的配套客户端，提供设备指纹注册、本地 HTTP 代理、客户端心跳、安全打开 Web 工作台、系统托盘常驻和国际化（中/英）能力。

---

## 技术栈

| 层 | 技术 |
|---|------|
| 壳 | Tauri v2 (Rust) |
| UI | 纯 HTML/CSS/JS（WebView 渲染，无打包工具） |
| HTTP 服务 | Axum（本地 HTTP） |
| HTTP 客户端 | Reqwest |
| 设备指纹 | ed25519-dalek + SHA-256 |
| 本地加密 | AES-256-GCM（`crypto.rs`，用于凭据本地保护） |
| i18n | 自研（LazyLock + HashMap / JSON） |
| 打包 | Tauri Bundler → DMG / MSI / AppImage / DEB |

---

## 项目结构

```
src/                          # 前端（WebView UI）
├── index.html                # Shell 页面（data-i18n 属性）
├── css/styles.css            # 深色主题样式
├── fonts/                    # 本地字体（Inter / JetBrains Mono）
├── js/
│   ├── app.js                # 入口：视图切换、启动流程、退出逻辑
│   ├── api.js                # Tauri IPC 封装
│   ├── i18n.js               # 前端 i18n 引擎
│   ├── avatar.js             # 默认头像 SVG 生成
│   └── views/
│       ├── login.js          # 登录表单逻辑
│       ├── panel.js          # 已连接面板：用户信息、日志、工作台入口
│       ├── background.js     # 动态背景（网格/粒子）
│       ├── wave.js           # 监视器圆球水波纹动画
│       └── settings.js       # 设置页（预留）
└── locales/
    ├── en.json               # 英文翻译
    └── zh.json               # 中文翻译

src-tauri/                    # Tauri Rust 后端
├── Cargo.toml
├── tauri.conf.json           # 窗口、托盘、CSP、打包目标
└── src/
    ├── main.rs               # 入口：窗口、托盘、生命周期、全局快捷键
    ├── lib.rs                # 模块声明
    ├── commands.rs           # 全部 Tauri IPC 命令
    ├── state.rs              # ProxyState, HeartbeatState
    ├── config.rs             # Config 结构体 + 读写（~/.devops-client/config.json，AES 加密）
    ├── fingerprint.rs        # ED25519 + SHA256 设备指纹
    ├── crypto.rs             # AES-256-GCM 本地加密/解密
    ├── proxy.rs              # Axum HTTP 本地代理（/ping）
    ├── auth.rs               # Reqwest HTTP 客户端 + 设备认证 API
    ├── error.rs              # AppError（带 i18n 消息）
    └── i18n.rs               # 语言检测 + 翻译表
```

---

## 命令

使用 [just](https://github.com/casey/just) 执行任务（见 `justfile`）：

```bash
just install    # pnpm install
just dev        # Tauri dev 模式（带热重载）
just debug      # Tauri dev --no-watch（快速启动测试）
just build      # 构建 macOS ARM64 DMG
just build-mac-x64  # 构建 macOS x64
just build-all  # CI：构建所有平台
just check      # cargo check（仅类型检查）
just fmt        # cargo fmt
just lint       # cargo clippy -- -D warnings
just clean      # 清理构建产物
just verify     # check + build
just run        # 打开构建后的 App
```

---

## 架构

### 模块职责

| 模块 | 职责 |
|------|------|
| `main.rs` | 入口：窗口创建、系统托盘、生命周期事件、窗口拖拽/关闭行为 |
| `commands.rs` | 所有 `#[tauri::command]` IPC 处理器 |
| `state.rs` | 状态管理：代理运行状态、心跳状态、当前 fingerprint |
| `config.rs` | 配置读写：`~/.devops-client/config.json`（AES-256-GCM 加密） |
| `fingerprint.rs` | 设备指纹：ED25519 密钥对 + SHA256 |
| `crypto.rs` | 本地 AES-256-GCM 加密，密钥由机器 UUID + 用户名派生 |
| `proxy.rs` | HTTP 本地代理：启动/停止、可用端口查找、`/ping` 响应 |
| `auth.rs` | API 客户端：`login-device`、`auto-login`、`device-status`、`create-exchange-token`、`get-user-info` |
| `error.rs` | 错误类型：带 i18n 消息的 AppError |
| `i18n.rs` | 国际化：Lang 枚举、语言检测、翻译函数 |

### 启动流程

```
main()
  → 生成/读取设备指纹
  → 检测系统语言
  → 创建主窗口（360×320，深色主题，无边框可拖拽）
  → 配置系统托盘（左键显示、菜单打开/退出）
  → 注册 IPC 命令
  → 前端加载 config.json，若有缓存 token 则尝试自动登录
```

### 登录流程

```
UI 表单 → API.doLogin(url, user, pass, deviceName)
  → Rust POST /api/auth/login-device（携带 fingerprint、os、clientVersion、deviceInfo）
  → 成功:
    → 保存 config.json（serverUrl、token、fingerprint、loginAt 等，AES 加密）
    → 启动本地 HTTP 代理 (127.0.0.1:{随机端口})
    → 启动心跳（每 10s POST /api/auth/device-status，Header X-Session-Id）
    → 切换到已连接面板
  → pending: 显示"需要管理员审批"
  → error: 显示服务端错误信息
```

### 打开工作台

```
点击面板"工作台"圆球
  → Rust POST /api/auth/create-exchange-token（X-Session-Id）
  → 获取 exchangeToken
  → 系统浏览器打开 /api/auth/exchange-token?exchangeToken=...&port=...
  → 后端校验原 session → 新建浏览器独立 session → 写入 SESSION_ID cookie
  → 按账号类型重定向：普通账号 → /dashboard，管理账号 → /index
```

点击期间设置 `_openingBrowser` 防抖，防止重复打开。

### 心跳与离线策略

- 心跳每 10 秒调用 `/api/auth/device-status`，携带 `X-Session-Id` 和 `fingerprint`
- 服务端校验 fingerprint 与 session 是否匹配：
  - 匹配：renewal 当前 session 及同 fingerprint 的所有 browser session
  - 不匹配：立即失效当前 session，并踢掉 fingerprint 所属用户的所有 session
- 客户端状态处理：
  - `Active` / `Pending`：失败计数清零
  - `Revoked`：发送 `device-revoked` 事件并退出应用
  - `SESSION_INVALID` / `FINGERPRINT_MISMATCH`：立即发送 `connection-lost` 事件
  - 其他 `Error` / `NotFound` 或网络错误：累计失败，达到 3 次后发送 `connection-lost`
- 前端收到 `connection-lost` 后调用 `_doLogout(false)`：停止 proxy、停止心跳、清除 token、返回登录页

### 安全模型

1. 用户通过密码登录 → 服务端创建 session，客户端保存 token
2. Agent 启动本地 HTTP 服务器 → 浏览器通过 `fetch(http://127.0.0.1:{port}/ping)` 验证 agent 存在
3. 心跳持续上报 → 设备被撤销或服务端 session 失效时客户端立即退出
4. Cookie 拷到其他机器 → 缺少本地 agent ping → 服务端触发 `/api/auth/device-offline` 并失效 session
5. fingerprint 与 session 不匹配 → 双杀：当前 session 失效 + fingerprint 所属用户全部 session 失效

### 指纹算法

```
raw = systemUUID + "/" + hostname + "/" + base64url(ed25519_pubkey)
fingerprint = SHA256(raw) → 64 位小写 hex
```

密钥持久化在 `~/.devops-client/device.key`（JSON: `{seed, pub}`）。

---

## 数据与配置

所有本地数据位于 `~/.devops-client/`：

```
.devops-client/
├── device.key          # ED25519 设备密钥（明文 JSON）
└── config.json         # AES-256-GCM 加密缓存（非明文，请勿手动编辑）
```

- `config.json` 使用 AES-256-GCM 加密，密钥由机器 UUID + 当前用户名派生，换机器无法解密。
- 如需迁移配置，必须重新登录。

---

## i18n

### 添加语言

1. 创建 `src/locales/{code}.json`（key 与 en.json 一致）
2. 在 `i18n.rs` 的 `i18n_map!` 宏中添加对应翻译
3. 在 `i18n.rs` 的 `detect_lang()` 中添加语言检测逻辑

### 前端用法

```html
<span data-i18n="login.title">Fallback text</span>
```

JS: `I18n.t('login.title')`

### Rust 用法

```rust
let msg = i18n::t(lang, "login.failed");
```

---

## 打包与跨平台

- `tauri.conf.json` 中 `bundle.targets` 使用 `"all"`，CI 各平台会自动构建该平台支持的所有安装包。
- macOS 本地构建：`just build`（ARM64）、`just build-mac-x64`（x64）。
- Windows / Linux 构建需在对应平台（或 CI）运行。

### macOS 图标缓存

替换 `src-tauri/icons/` 后若 Dock/启动台仍显示旧图标：

```bash
rm -rf /private/var/folders/*/*/*/com.apple.dock.iconcache
rm -rf /private/var/folders/*/*/*/com.apple.iconservices.store
killall Dock
killall Finder
```

---

## 注意事项

- Public repo — 代码中**禁止**包含公司/服务器/内部信息
- 服务器地址由用户手动输入，加密缓存于 `~/.devops-client/config.json`
- 关闭窗口 = 隐藏到托盘，只有托盘菜单"退出"才真正退出
- 前端无打包工具，JS 通过全局变量（`API` / `I18n` / `App` / `LoginView` / `Panel` / `Wave` / `Background` / `AvatarUtil`）通信
- 心跳与 proxy 必须在登录成功后启动，登出/连接丢失时务必停止，避免端口占用和无效请求
- 本地代理使用纯 HTTP 并仅绑定 `127.0.0.1`，不对外提供服务，无需 TLS 证书
