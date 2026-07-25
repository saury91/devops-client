# CLAUDE.md

DevOps Client — Tauri v2 桌面设备认证代理。提供本地 HTTPS 服务 + 系统托盘 + 国际化（中/英）。

## 技术栈

| 层 | 技术 |
|---|------|
| 壳 | Tauri v2 (Rust) |
| UI | 纯 HTML/CSS/JS（WebView 渲染） |
| HTTP 服务 | Axum + Rustls（本地 HTTPS） |
| HTTP 客户端 | Reqwest |
| 加密 | ed25519-dalek, SHA-256 |
| i18n | 自研（LazyLock + HashMap / JSON） |
| 打包 | Tauri Bundler → DMG / MSI / AppImage |

## 项目结构

```
src/                          # 前端（WebView UI）
├── index.html                # Shell 页面（data-i18n 属性）
├── css/styles.css            # 暗色主题
├── js/
│   ├── app.js                # 入口、视图切换、启动
│   ├── api.js                # Tauri IPC 封装
│   ├── i18n.js               # i18n 引擎（加载 locale JSON）
│   └── views/
│       ├── login.js          # 登录表单逻辑
│       └── panel.js          # 已连接面板逻辑
└── locales/
    ├── en.json               # 英文翻译
    └── zh.json               # 中文翻译

src-tauri/                    # Tauri Rust 后端
├── Cargo.toml
├── tauri.conf.json           # 窗口 420×560, 托盘, CSP
└── src/
    ├── main.rs               # 入口：窗口、托盘、生命周期
    ├── lib.rs                # 模块声明
    ├── commands.rs           # 全部 Tauri IPC 命令
    ├── state.rs              # ProxyState, HeartbeatState
    ├── config.rs             # Config 结构体 + 读写
    ├── fingerprint.rs        # ED25519 + SHA256 设备指纹
    ├── cert.rs               # 自签 CA + localhost TLS
    ├── proxy.rs              # Axum HTTPS 代理 (/ping)
    ├── auth.rs               # Reqwest HTTP 客户端
    ├── error.rs              # AppError（带 i18n 消息）
    └── i18n.rs               # 语言检测 + 翻译表
```

## 命令

```bash
just install    # pnpm install
just dev        # Tauri dev 模式
just build      # 构建 macOS ARM64 DMG
just check      # cargo check（仅类型检查）
just fmt        # cargo fmt
just lint       # cargo clippy
just clean      # 清理
```

## 架构

### 模块职责

| 模块 | 职责 |
|------|------|
| `main.rs` | 入口：窗口创建、托盘配置、生命周期事件 |
| `commands.rs` | 所有 `#[tauri::command]` IPC 处理器 |
| `state.rs` | 状态管理：代理运行状态、心跳状态 |
| `config.rs` | 配置读写：`~/.devops-client/config.json` |
| `fingerprint.rs` | 设备指纹：ED25519 密钥 + SHA256 |
| `cert.rs` | 证书管理：CA 生成、localhost 证书、信任存储 |
| `proxy.rs` | HTTPS 代理：启动/停止、可用端口查找 |
| `auth.rs` | API 客户端：login-device、device-status |
| `error.rs` | 错误类型：带 i18n 消息的 AppError |
| `i18n.rs` | 国际化：Lang 枚举、语言检测、翻译函数 |

### 启动流程

```
main() → 生成指纹 + 自签证书
       → 检测系统语言
       → 创建窗口 (420×560, 暗色主题)
       → 配置系统托盘（关闭=隐藏、左键=显示、菜单=打开/退出）
       → 注册 IPC 命令
```

### 登录流程

```
UI 表单 → API.doLogin(url, user, pass, hostname)
  → Rust POST /api/auth/login-device
  → 成功:
    → 保存 config.json
    → 启动 HTTPS 代理 (127.0.0.1:{random})
    → 启动心跳 (每 10s GET /api/auth/device-status)
    → 打开浏览器: /api/auth/exchange-token?token=X&port=Y
    → 切换到已连接面板
  → pending: 显示"需要管理员审批"
  → error: 显示错误信息
```

### 安全模型

1. 用户通过密码登录 → 服务端签发 session
2. Agent 启动本地 HTTPS 服务器 → 浏览器通过 `fetch(https://127.0.0.1:{port}/ping)` 验证 agent 存在
3. 心跳每 10s 上报 → 管理员撤销时 agent 立即退出
4. Cookie 拷到其他机器 → localhost ping 失败 → session 失效

### 指纹算法

```
raw = systemUUID + "/" + hostname + "/" + base64url(ed25519_pubkey)
fingerprint = SHA256(raw) → 64 位小写 hex
```

密钥持久化在 `~/.devops-client/device.key`（JSON: `{seed, pub}`）。

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

## 注意事项

- Public repo — 代码中**禁止**包含公司/服务器/内部信息
- 服务器地址由用户手动输入，缓存于 `~/.devops-client/config.json`
- 自签证书首次启动时写入系统信任存储（macOS 弹授权框）
- 关闭窗口 = 隐藏到托盘，只有托盘菜单"退出"才真正退出
- 前端无打包工具，JS 通过全局变量（API / I18n / App / LoginView / PanelView）通信
